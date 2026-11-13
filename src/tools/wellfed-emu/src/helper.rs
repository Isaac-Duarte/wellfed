//! The vendor UART flash helper, executing on synthetic NOR.
//!
//! The pinned helper runs from modeled RAM at `0x82020` as it does after the
//! loader uploads it. Its command parser, packet checks and program wrapper
//! execute as ARM; the ROM UART and flash calls it makes are modeled here.
//! Each modeled erase and program is refused outside the sectors a test
//! allows, which stay inside one OTA bank and never touch the factory region.
//! An unreviewed ROM call stops the session, as a real fault would.
//!
//! The host side is serial-shaped: [`Helper::write`] supplies bytes the
//! helper will read, and [`Helper::read`] runs the helper until it has output.

use std::collections::VecDeque;

use unicorn_engine::{Arch, ArmCpuModel, Mode, Prot, RegisterARM, Unicorn};
use wellfed_formats::canonical::sha256;
use wellfed_formats::require;

use crate::{BANKS, FLASH_BYTES, HELPER_SHA256, Result, SECTOR};

/// A helper session refused: an unpinned input, a scope outside one bank,
/// or a ROM call the model does not expect. Any of these stops the session,
/// as a real fault would.
#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("Pinned helper required")]
    UnpinnedHelper,
    #[error("Synthetic snapshot must be 8 MiB")]
    SnapshotSize { bytes: usize },
    #[error("Synthetic scope must be one bank")]
    ScopeBank { bank: u8 },
    #[error("Synthetic erase/program scope must stay in its bank")]
    ScopeSectors,
    #[error("Helper produced no bounded response")]
    NoResponse,
    #[error("Helper waiting without an expected response")]
    Waiting,
    #[error("UART getc without supplied host input")]
    NoHostInput,
    #[error("UART transmit escaped helper RAM")]
    TransmitEscapes { pointer: u32, length: u32 },
    #[error("Unexpected status-read command or buffer")]
    StatusRead { opcode: u32, length: u32 },
    #[error("Unexpected UART-writable polling")]
    WritablePolling,
    #[error("Erase escapes allowed application sectors")]
    EraseEscapes { offset: u32 },
    #[error("Program escapes allowed application sectors")]
    ProgramEscapes { offset: u32, length: u32 },
    #[error("Program buffer outside helper RAM")]
    ProgramBuffer { pointer: u32, length: u32 },
    #[error("Memcpy escaped helper RAM")]
    MemcpyEscapes,
    #[error("Memset escaped helper RAM")]
    MemsetEscapes,
    #[error("Unexpected helper UART initialization")]
    UartInitialization,
    #[error("Unexpected OTF setup")]
    OtfSetup,
    #[error("Unexpected backup register")]
    BackupRegister,
    #[error(
        "Unexpected helper destination (unreviewed flash/register operation forbidden): {pc:#x}"
    )]
    UnreviewedDestination { pc: u64 },
    #[error("Out-of-range register")]
    RegisterRange,
}

const FLASH_BASE: u64 = 0x0800_0000;
const CODE: u64 = 0x8_2020;
const CODE_END: u64 = 0x8_3030;
const RAM: std::ops::Range<u64> = 0x8_0000..0x9_0000;
const HELPER_BYTES: usize = 4688;
const BANK_BYTES: u32 = 0x15_0000;

/// One modeled flash call the helper made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Erase { offset: u32 },
    Program { offset: u32, bytes: u32 },
    ReadStatus { opcode: u8 },
}

/// What a session may touch, beyond the snapshot it starts from.
#[derive(Debug, Clone)]
pub struct Scope {
    /// The bank the allowed sectors lie in: 0 or 1.
    pub bank: u8,
    /// Sector offsets the helper may erase and program.
    pub sectors: Vec<u32>,
    /// The 05/35/15 status bytes; `None` refuses status reads.
    pub status: Option<[u8; 3]>,
}

#[derive(Default)]
struct State {
    incoming: VecDeque<u8>,
    outgoing: Vec<u8>,
    sectors: Vec<u32>,
    status: Option<[u8; 3]>,
    calls: Vec<Call>,
    empty_polls: u32,
    error: Option<crate::Error>,
}

pub struct Helper<'a> {
    uc: Unicorn<'a, State>,
    started: bool,
    host_tx: Vec<u8>,
}

impl Helper<'_> {
    /// The helper with `snapshot` (8 MiB) as flash; erases and programs are
    /// confined to `scope`.
    pub fn new(helper: &[u8], snapshot: &[u8], scope: Scope) -> Result<Self> {
        require(
            helper.len() == HELPER_BYTES && sha256(helper) == HELPER_SHA256,
            || HelperError::UnpinnedHelper,
        )?;
        Self::load(helper, snapshot, scope)
    }

    /// `helper` loaded where the loader puts it, unchecked against the pin:
    /// [`Self::new`] checks it first. The unit tests load a stand-in here to
    /// reach every modeled call's refusal without the private helper.
    fn load(helper: &[u8], snapshot: &[u8], scope: Scope) -> Result<Self> {
        require(snapshot.len() == FLASH_BYTES, || {
            HelperError::SnapshotSize {
                bytes: snapshot.len(),
            }
        })?;
        let start = *BANKS
            .get(usize::from(scope.bank))
            .ok_or(HelperError::ScopeBank { bank: scope.bank })?;
        require(
            !scope.sectors.is_empty()
                && scope
                    .sectors
                    .iter()
                    .all(|s| (start..start + BANK_BYTES).contains(s) && s % SECTOR == 0),
            || HelperError::ScopeSectors,
        )?;

        let state = State {
            sectors: scope.sectors,
            status: scope.status,
            ..State::default()
        };
        let mut uc = Unicorn::new_with_data(Arch::ARM, Mode::THUMB | Mode::MCLASS, state)?;
        uc.ctl_set_cpu_model(ArmCpuModel::CORTEX_M4 as i32)?;
        uc.mem_map(0, 0x10000, Prot::ALL)?;
        uc.mem_map(RAM.start, RAM.end - RAM.start, Prot::ALL)?;
        uc.mem_write(0x8_2000, helper)?;
        uc.mem_map(FLASH_BASE, 0x80_0000, Prot::READ | Prot::EXEC)?;
        uc.mem_write(FLASH_BASE, snapshot)?;
        uc.mem_map(0x4800_0000, 0x1000, Prot::ALL)?;
        uc.mem_map(0xE000_E000, 0x2000, Prot::ALL)?;
        uc.reg_write(RegisterARM::SP, 0x8_FF00)?;

        // The helper's own code runs unhooked; everything it calls is modeled.
        for (begin, end) in [(0, CODE - 1), (CODE_END, u64::from(u32::MAX))] {
            uc.add_code_hook(begin, end, |uc, pc, _| {
                if uc.get_data().error.is_some() {
                    return;
                }
                if let Err(error) = call(uc, pc) {
                    uc.get_data_mut().error = Some(error);
                    let _ = uc.emu_stop();
                }
            })?;
        }
        Ok(Self {
            uc,
            started: false,
            host_tx: Vec::new(),
        })
    }

    /// Supply host bytes for the helper to read.
    pub fn write(&mut self, data: &[u8]) -> usize {
        self.uc.get_data_mut().incoming.extend(data);
        self.host_tx.extend_from_slice(data);
        data.len()
    }

    /// Up to `count` output bytes, running the helper first if it has none.
    pub fn read(&mut self, count: usize) -> Result<Vec<u8>> {
        if self.uc.get_data().outgoing.is_empty() {
            self.drive()?;
        }
        let outgoing = &mut self.uc.get_data_mut().outgoing;
        let count = count.min(outgoing.len());
        Ok(outgoing.drain(..count).collect())
    }

    pub fn in_waiting(&self) -> usize {
        self.uc.get_data().outgoing.len()
    }

    /// Everything the host sent, in order.
    pub fn host_tx(&self) -> &[u8] {
        &self.host_tx
    }

    pub fn clear_host_tx(&mut self) {
        self.host_tx.clear();
    }

    pub fn calls(&self) -> &[Call] {
        &self.uc.get_data().calls
    }

    /// The whole synthetic flash as it is now.
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        Ok(self.uc.mem_read_as_vec(FLASH_BASE, FLASH_BYTES)?)
    }

    /// Replace flash bytes directly, as a fixture arranging a starting state.
    pub fn seed(&mut self, offset: u32, data: &[u8]) -> Result<()> {
        Ok(self.uc.mem_write(FLASH_BASE + u64::from(offset), data)?)
    }

    /// Run the helper until it waits for input with output pending.
    fn drive(&mut self) -> Result<()> {
        let pc = if self.started {
            self.uc.reg_read(RegisterARM::PC)?
        } else {
            CODE
        };
        self.started = true;
        self.uc.get_data_mut().empty_polls = 0;

        // The instruction count is the only bound: a wall-clock timeout is
        // what a loaded host trips, not a runaway helper.
        let ran = self.uc.emu_start(pc | 1, CODE_END, 0, 500_000);
        if let Some(error) = self.uc.get_data_mut().error.take() {
            return Err(error);
        }
        ran?;
        require(!self.uc.get_data().outgoing.is_empty(), || {
            HelperError::NoResponse.into()
        })
    }
}

/// Model one ROM call (or refuse it), then return to the helper.
fn call(uc: &mut Unicorn<State>, pc: u64) -> Result<()> {
    let args = [
        reg(uc, RegisterARM::R0)?,
        reg(uc, RegisterARM::R1)?,
        reg(uc, RegisterARM::R2)?,
    ];
    match pc {
        // UART readable poll: pause with output pending, so the host reads
        // before the helper waits on input it has not been sent yet.
        0x6220 => {
            let state = uc.get_data_mut();
            if state.incoming.is_empty() && !state.outgoing.is_empty() {
                uc.emu_stop()?;
                return Ok(());
            }
            state.empty_polls = if state.incoming.is_empty() {
                state.empty_polls + 1
            } else {
                0
            };
            require(state.empty_polls < 100, || HelperError::Waiting)?;
            let ready = u32::from(!state.incoming.is_empty());
            ret(uc, ready)
        }
        0x6250 => {
            let byte = uc.get_data_mut().incoming.pop_front();
            let byte = byte.ok_or(HelperError::NoHostInput)?;
            ret(uc, u32::from(byte))
        }
        0x6274 => {
            uc.get_data_mut().outgoing.push(args[0].to_le_bytes()[0]);
            ret(uc, 1)
        }
        0x628C => {
            let [pointer, length, _] = args;
            require(within(pointer, length) && length <= 1032, || {
                HelperError::TransmitEscapes { pointer, length }
            })?;
            let data = uc.mem_read_as_vec(u64::from(pointer), length as usize)?;
            uc.get_data_mut().outgoing.extend(data);
            ret(uc, length)
        }
        0x1D1C => {
            let [opcode, length, pointer] = args;
            let codes = [0x05_u8, 0x35, 0x15];
            let index = codes.iter().position(|&o| u32::from(o) == opcode);
            let value = uc
                .get_data()
                .status
                .zip(index)
                .map(|(s, i)| (s[i], codes[i]));
            let Some((value, code)) = value.filter(|_| length == 1 && within(pointer, 1)) else {
                return Err(HelperError::StatusRead { opcode, length }.into());
            };
            uc.mem_write(u64::from(pointer), &[value])?;
            uc.get_data_mut()
                .calls
                .push(Call::ReadStatus { opcode: code });
            ret(uc, 0)
        }
        0x6238 => {
            require(uc.get_data().status.is_some(), || {
                HelperError::WritablePolling
            })?;
            ret(uc, 1)
        }
        0x2146 => {
            let [mode, offset, _] = args;
            require(mode == 2 && uc.get_data().sectors.contains(&offset), || {
                HelperError::EraseEscapes { offset }
            })?;
            uc.mem_write(FLASH_BASE + u64::from(offset), &[0xFF; SECTOR as usize])?;
            uc.get_data_mut().calls.push(Call::Erase { offset });
            ret(uc, 0)
        }
        0x2048 => {
            let [offset, length, pointer] = args;
            require(
                length == 8
                    && uc.get_data().sectors.contains(&(offset / SECTOR * SECTOR))
                    && offset % SECTOR + length <= SECTOR,
                || HelperError::ProgramEscapes { offset, length },
            )?;
            require(within(pointer, length), || HelperError::ProgramBuffer {
                pointer,
                length,
            })?;
            // NOR programming only clears bits.
            let address = FLASH_BASE + u64::from(offset);
            let existing = uc.mem_read_as_vec(address, 8)?;
            let data = uc.mem_read_as_vec(u64::from(pointer), 8)?;
            let programmed: Vec<u8> = existing.iter().zip(&data).map(|(a, b)| a & b).collect();
            uc.mem_write(address, &programmed)?;
            uc.get_data_mut().calls.push(Call::Program {
                offset,
                bytes: length,
            });
            ret(uc, 0)
        }
        0x6590 => {
            let [destination, source, length] = args;
            require(
                length <= 4096 && within(destination, length) && within(source, length),
                || HelperError::MemcpyEscapes,
            )?;
            let data = uc.mem_read_as_vec(u64::from(source), length as usize)?;
            uc.mem_write(u64::from(destination), &data)?;
            ret(uc, destination)
        }
        0x66AC => {
            let [destination, value, length] = args;
            require(length <= 4096 && within(destination, length), || {
                HelperError::MemsetEscapes
            })?;
            let fill = vec![value.to_le_bytes()[0]; length as usize];
            uc.mem_write(u64::from(destination), &fill)?;
            ret(uc, destination)
        }
        0x61A0 => {
            require(args == [2, 0, 115_200], || HelperError::UartInitialization)?;
            uc.mem_write(0x8_0660, &0x4000_8000u32.to_le_bytes())?;
            ret(uc, 0)
        }
        0x3594 => {
            require(args[0] == 0, || HelperError::OtfSetup)?;
            uc.mem_write(0x4800_0600, &[0; 4])?;
            ret(uc, 0)
        }
        0x1434 => {
            require(args[0] == 0, || HelperError::BackupRegister)?;
            ret(uc, 0)
        }
        // Diagnostic, delay, UART baud and busy models only.
        0x0474 | 0x14C2 | 0x1504 | 0x40B0 | 0x43D4 => ret(uc, 0),
        _ => Err(HelperError::UnreviewedDestination { pc }.into()),
    }
}

/// Return `value` from the modeled call to its caller.
fn ret(uc: &mut Unicorn<State>, value: u32) -> Result<()> {
    uc.reg_write(RegisterARM::R0, u64::from(value))?;
    let back = uc.reg_read(RegisterARM::LR)?;
    uc.reg_write(RegisterARM::PC, back)?;
    Ok(())
}

/// `[pointer, pointer + length)` lies in helper RAM.
fn within(pointer: u32, length: u32) -> bool {
    let (pointer, length) = (u64::from(pointer), u64::from(length));
    RAM.start <= pointer && pointer + length <= RAM.end
}

fn reg(uc: &Unicorn<State>, register: RegisterARM) -> Result<u32> {
    Ok(u32::try_from(uc.reg_read(register)?).map_err(|_| HelperError::RegisterRange)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    /// A sector of bank 1 a test may erase and program, and one beside it.
    const ALLOWED: u32 = 0x18_6000 + 4 * SECTOR;
    const OTHER: u32 = ALLOWED + SECTOR;
    /// Helper RAM a modeled call's buffer may use.
    const BUFFER: u32 = 0x8_1000;

    const ERASE: u64 = 0x2146;
    const PROGRAM: u64 = 0x2048;
    const TRANSMIT: u64 = 0x628C;
    const STATUS: u64 = 0x1D1C;
    const WRITABLE: u64 = 0x6238;
    const MEMCPY: u64 = 0x6590;
    const MEMSET: u64 = 0x66AC;

    fn scope(status: Option<[u8; 3]>) -> Scope {
        Scope {
            bank: 1,
            sectors: vec![ALLOWED],
            status,
        }
    }

    /// A stand-in for the pinned helper over erased flash: its code is
    /// never run, only the modeled ROM calls it would make.
    fn helper(status: Option<[u8; 3]>) -> Helper<'static> {
        Helper::load(&[0; HELPER_BYTES], &vec![0xFF; FLASH_BYTES], scope(status)).unwrap()
    }

    impl Helper<'_> {
        /// Branch to the modeled ROM routine at `pc` with `args` in r0-r2,
        /// as the helper's code does, and return what it left in r0.
        fn call_rom(&mut self, pc: u64, args: [u32; 3]) -> Result<u32> {
            let registers = [RegisterARM::R0, RegisterARM::R1, RegisterARM::R2];
            for (register, value) in registers.into_iter().zip(args) {
                self.uc.reg_write(register, u64::from(value))?;
            }
            // The model returns to the caller's link register: back to the
            // helper's entry, where the run stops.
            self.uc.reg_write(RegisterARM::LR, CODE | 1)?;

            let ran = self.uc.emu_start(pc | 1, CODE, 0, 100);
            if let Some(error) = self.uc.get_data_mut().error.take() {
                return Err(error);
            }
            ran?;
            reg(&self.uc, RegisterARM::R0)
        }

        fn flash(&self, offset: u32, len: usize) -> Vec<u8> {
            self.uc
                .mem_read_as_vec(FLASH_BASE + u64::from(offset), len)
                .unwrap()
        }

        fn ram(&mut self, at: u32, data: &[u8]) {
            self.uc.mem_write(u64::from(at), data).unwrap();
        }
    }

    fn refused(result: Result<u32>) -> HelperError {
        match result {
            Err(Error::Helper(error)) => error,
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn only_the_pinned_helper_runs() {
        let snapshot = vec![0xFF; FLASH_BYTES];
        for stand_in in [vec![0; HELPER_BYTES], vec![0; HELPER_BYTES - 1]] {
            let error = Helper::new(&stand_in, &snapshot, scope(None)).err();
            assert!(
                matches!(error, Some(Error::Helper(HelperError::UnpinnedHelper))),
                "{error:?}"
            );
        }
    }

    /// The session's scope is checked before anything runs: an 8 MiB
    /// snapshot, a bank that exists, and sectors inside it on sector
    /// boundaries.
    #[test]
    fn a_scope_outside_one_bank_is_refused() {
        let load = |snapshot: &[u8], scope: Scope| {
            Helper::load(&[0; HELPER_BYTES], snapshot, scope)
                .err()
                .map(|error| match error {
                    Error::Helper(error) => error,
                    other => panic!("{other:?}"),
                })
        };
        let flash = vec![0xFF; FLASH_BYTES];

        assert!(matches!(
            load(&flash[..FLASH_BYTES - 1], scope(None)),
            Some(HelperError::SnapshotSize { bytes }) if bytes == FLASH_BYTES - 1
        ));
        let bank = |bank, sectors| Scope {
            bank,
            sectors,
            status: None,
        };
        assert!(matches!(
            load(&flash, bank(2, vec![ALLOWED])),
            Some(HelperError::ScopeBank { bank: 2 })
        ));
        for sectors in [
            vec![],
            vec![ALLOWED + 8],
            vec![BANKS[0]],
            vec![BANKS[1] + BANK_BYTES],
        ] {
            assert!(
                matches!(
                    load(&flash, bank(1, sectors.clone())),
                    Some(HelperError::ScopeSectors)
                ),
                "{sectors:x?}"
            );
        }
        assert!(load(&flash, bank(1, vec![BANKS[1]])).is_none());
    }

    #[test]
    fn an_erase_runs_only_on_an_allowed_sector() {
        let mut helper = helper(None);
        helper.seed(ALLOWED, &[0; 64]).unwrap();

        assert_eq!(helper.call_rom(ERASE, [2, ALLOWED, 0]).unwrap(), 0);
        assert_eq!(helper.flash(ALLOWED, 64), [0xFF; 64]);
        assert_eq!(helper.calls(), [Call::Erase { offset: ALLOWED }]);

        assert!(matches!(
            refused(helper.call_rom(ERASE, [2, OTHER, 0])),
            HelperError::EraseEscapes { offset: OTHER }
        ));
        // Only the sector erase mode.
        let mut helper = super::tests::helper(None);
        assert!(matches!(
            refused(helper.call_rom(ERASE, [1, ALLOWED, 0])),
            HelperError::EraseEscapes { offset: ALLOWED }
        ));
        assert!(helper.calls().is_empty());
    }

    /// A program clears bits only, as NOR does, eight bytes at a time,
    /// inside one allowed sector, from helper RAM.
    #[test]
    fn a_program_ands_eight_bytes_into_an_allowed_sector() {
        let mut helper = helper(None);
        helper.seed(ALLOWED + 8, &[0xF0; 8]).unwrap();
        helper.ram(BUFFER, &[0x3C; 8]);

        assert_eq!(
            helper.call_rom(PROGRAM, [ALLOWED + 8, 8, BUFFER]).unwrap(),
            0
        );
        assert_eq!(helper.flash(ALLOWED + 8, 8), [0x30; 8]);
        assert_eq!(
            helper.calls(),
            [Call::Program {
                offset: ALLOWED + 8,
                bytes: 8
            }]
        );

        for (offset, length) in [
            (ALLOWED, 4),
            (ALLOWED, 16),
            (OTHER, 8),
            // Across the end of the allowed sector into the next.
            (ALLOWED + SECTOR - 4, 8),
        ] {
            let mut helper = super::tests::helper(None);
            let error = refused(helper.call_rom(PROGRAM, [offset, length, BUFFER]));
            assert!(
                matches!(error, HelperError::ProgramEscapes { offset: o, length: l } if o == offset && l == length),
                "{offset:#x}+{length}: {error:?}"
            );
            assert!(helper.calls().is_empty());
        }

        // The data from flash, not the helper's RAM.
        let mut helper = super::tests::helper(None);
        let flash = 0x0800_0000;
        assert!(matches!(
            refused(helper.call_rom(PROGRAM, [ALLOWED, 8, flash])),
            HelperError::ProgramBuffer {
                pointer: 0x0800_0000,
                length: 8
            }
        ));
        assert_eq!(helper.flash(ALLOWED, 8), [0xFF; 8]);
    }

    #[test]
    fn a_transmit_sends_only_helper_ram() {
        let mut helper = helper(None);
        helper.ram(BUFFER, b"OK");
        assert_eq!(helper.call_rom(TRANSMIT, [BUFFER, 2, 0]).unwrap(), 2);
        assert_eq!(helper.in_waiting(), 2);

        for (pointer, length) in [(0x0800_0000, 2), (BUFFER, 1033), (0x8_FFFF, 2)] {
            let mut helper = super::tests::helper(None);
            assert!(matches!(
                refused(helper.call_rom(TRANSMIT, [pointer, length, 0])),
                HelperError::TransmitEscapes { pointer: p, length: l } if p == pointer && l == length
            ));
        }
    }

    /// Status reads and writable polling run only when the session gives
    /// status bytes, and only the three reviewed opcodes, one byte each.
    #[test]
    fn status_reads_need_a_status_and_a_reviewed_opcode() {
        let mut helper = helper(None);
        assert!(matches!(
            refused(helper.call_rom(STATUS, [0x05, 1, BUFFER])),
            HelperError::StatusRead {
                opcode: 0x05,
                length: 1
            }
        ));
        let mut helper = super::tests::helper(None);
        assert!(matches!(
            refused(helper.call_rom(WRITABLE, [0, 0, 0])),
            HelperError::WritablePolling
        ));

        let mut helper = super::tests::helper(Some([0x02, 0x40, 0x60]));
        assert_eq!(helper.call_rom(STATUS, [0x35, 1, BUFFER]).unwrap(), 0);
        assert_eq!(
            helper.uc.mem_read_as_vec(u64::from(BUFFER), 1).unwrap(),
            [0x40]
        );
        assert_eq!(helper.calls(), [Call::ReadStatus { opcode: 0x35 }]);
        assert_eq!(helper.call_rom(WRITABLE, [0, 0, 0]).unwrap(), 1);
        for (opcode, length, pointer) in [(0x9F, 1, BUFFER), (0x05, 2, BUFFER), (0x05, 1, 0)] {
            assert!(matches!(
                refused(helper.call_rom(STATUS, [opcode, length, pointer])),
                HelperError::StatusRead { opcode: o, length: l } if o == opcode && l == length
            ));
        }
    }

    #[test]
    fn copies_and_fills_stay_in_helper_ram() {
        let mut helper = helper(None);
        helper.ram(BUFFER, b"abcd");
        assert_eq!(
            helper.call_rom(MEMCPY, [BUFFER + 16, BUFFER, 4]).unwrap(),
            BUFFER + 16
        );
        assert_eq!(
            helper
                .uc
                .mem_read_as_vec(u64::from(BUFFER + 16), 4)
                .unwrap(),
            b"abcd"
        );
        assert_eq!(helper.call_rom(MEMSET, [BUFFER, 0x5A, 4]).unwrap(), BUFFER);
        assert_eq!(
            helper.uc.mem_read_as_vec(u64::from(BUFFER), 4).unwrap(),
            b"ZZZZ"
        );

        for args in [
            [0x0800_0000, BUFFER, 4],
            [BUFFER, 0x0800_0000, 4],
            [BUFFER, BUFFER, 4097],
        ] {
            assert!(matches!(
                refused(helper.call_rom(MEMCPY, args)),
                HelperError::MemcpyEscapes
            ));
        }
        for args in [[0x0800_0000, 0, 4], [BUFFER, 0, 4097]] {
            assert!(matches!(
                refused(helper.call_rom(MEMSET, args)),
                HelperError::MemsetEscapes
            ));
        }
    }

    /// The setup calls the helper makes once are modeled only with the
    /// arguments the pinned helper passes; anything else, and any routine
    /// no review covers, stops the session.
    #[test]
    fn unreviewed_setup_and_destinations_stop_the_session() {
        let mut helper = helper(None);
        assert!(matches!(
            refused(helper.call_rom(0x61A0, [2, 0, 9_600])),
            HelperError::UartInitialization
        ));
        assert!(matches!(
            refused(helper.call_rom(0x3594, [1, 0, 0])),
            HelperError::OtfSetup
        ));
        assert!(matches!(
            refused(helper.call_rom(0x1434, [1, 0, 0])),
            HelperError::BackupRegister
        ));
        assert!(matches!(
            refused(helper.call_rom(0x6250, [0, 0, 0])),
            HelperError::NoHostInput
        ));
        assert!(matches!(
            refused(helper.call_rom(0x1234, [0, 0, 0])),
            HelperError::UnreviewedDestination { pc: 0x1234 }
        ));
        assert!(helper.calls().is_empty());

        let mut helper = super::tests::helper(None);
        assert_eq!(helper.call_rom(0x61A0, [2, 0, 115_200]).unwrap(), 0);
        helper.write(&[0x42]);
        assert_eq!(helper.call_rom(0x6250, [0, 0, 0]).unwrap(), 0x42);
    }
}
