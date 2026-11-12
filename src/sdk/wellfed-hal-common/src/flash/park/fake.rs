//! Host stand-ins for the IPC registers the park handshake runs over, for
//! the tests of every crate that handshakes or arms.
//!
//! [`Ipc`] is both cores' USR words in one register model per thread: a
//! test steps the KM0 side (`Ipc<true>`) and the KM4 side (`Ipc<false>`)
//! against each other. [`Unreached`] is for a caller that must never
//! handshake (its hooks are already armed), and [`Refusing`] a KM0 that
//! takes no hello.

use std::cell::RefCell;

use super::{KM0_IPC, KM4_IPC, Mmio, READY, handshake, publish_raw};

/// The USR words each core's model keeps: the one count the register
/// offsets and the array length both come from.
const USR_WORDS: u32 = 12;
const WORDS: usize = USR_WORDS as usize;
/// Where USR word 0 sits in a core's IPC block, and where the modelled
/// words end.
const USR: u32 = 0x20;
const USR_END: u32 = USR + 4 * USR_WORDS;

thread_local! {
    /// USR words 0..11 per core: index 0 KM0, 1 KM4.
    static REGS: RefCell<[[u32; WORDS]; 2]> = const { RefCell::new([[0; WORDS]; 2]) };
}

/// Clear both cores' words, as at power-on.
pub fn reset() {
    REGS.with(|r| *r.borrow_mut() = [[0; WORDS]; 2]);
}

/// USR word `index` of `core` (0 KM0, 1 KM4).
pub fn usr(core: usize, index: usize) -> u32 {
    REGS.with(|r| r.borrow()[core][index])
}

/// Set USR word `index` of `core`, as the other side would.
pub fn set_usr(core: usize, index: usize, value: u32) {
    REGS.with(|r| r.borrow_mut()[core][index] = value);
}

fn locate(address: u32) -> (usize, usize) {
    let core = match address & 0xffff_f000 {
        KM0_IPC => 0,
        KM4_IPC => 1,
        _ => panic!("unexpected address {address:#x}"),
    };
    let offset = address & 0xfff;
    assert!(
        (USR..USR_END).contains(&offset) && offset & 3 == 0,
        "not a USR word: {address:#x}"
    );
    (core, ((offset - USR) / 4) as usize)
}

/// One core's view of the shared register model.
pub struct Ipc<const KM0: bool>;

impl<const KM0: bool> Mmio for Ipc<KM0> {
    const KM0: bool = KM0;
    fn load(address: u32) -> u32 {
        let (core, index) = locate(address);
        usr(core, index)
    }
    fn store(address: u32, value: u32) {
        let (core, index) = locate(address);
        set_usr(core, index, value);
    }
    fn barrier() {}
    fn mask() -> u32 {
        0
    }
    fn halt() -> ! {
        panic!("halt")
    }
}

/// KM4 with its hooks already armed, as after confirmation: any handshake
/// is a test's mistake.
pub struct Unreached;

impl Mmio for Unreached {
    const KM0: bool = false;
    fn load(_: u32) -> u32 {
        unreachable!("no arming with the hooks armed")
    }
    fn store(_: u32, _: u32) {
        unreachable!("no arming with the hooks armed")
    }
    fn barrier() {}
    fn mask() -> u32 {
        0
    }
    fn halt() -> ! {
        panic!("halt")
    }
}

/// A KM0-side view in which `hello` is refused, as with no KM4 authority to
/// announce: whatever handshakes ends at once.
pub struct Refusing;

impl Mmio for Refusing {
    const KM0: bool = true;
    fn load(_: u32) -> u32 {
        0
    }
    fn store(_: u32, _: u32) {}
    fn barrier() {}
    fn mask() -> u32 {
        0
    }
    fn halt() -> ! {
        panic!("halt")
    }
}

/// KM0 peer: republish READY for the answered authority with `peer`, as a
/// rebooted peer would, for the authority's tests of a lost peer.
pub fn reannounce<M: Mmio>(peer: u64, answered: u64) -> bool {
    if !M::KM0 || peer == 0 || answered == 0 {
        return false;
    }
    publish_raw::<M>(&handshake(answered, peer), READY);
    true
}
