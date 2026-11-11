//! The dual-slot store every record on NOR but the event ring is kept in:
//! the SDK's feed journal, seed and update store here, and the product's
//! settings, Wi-Fi credentials and client table in
//! `wellfed_feeder::store`. Each store keeps one little-endian record in
//! each of two alternating erase units and supplies its own field layout
//! and recovery policy; [`DualSlot`] owns the header, the checks, the write
//! order and what a unit can hold after a cut.
//!
//! # The record
//!
//! | Bytes | Field |
//! | --- | --- |
//! | 0..8 | the store's magic |
//! | 8 | format major |
//! | 9 | format minor |
//! | 10..12 | record length N (u16) |
//! | 12..N-8 | the store's fields; unused bytes zero |
//! | N-8..N-4 | CRC-32/ISO-HDLC of bytes 0..N-8 |
//! | N-4..N | `CMIT`, programmed last |
//!
//! # Versions
//!
//! Bank 0 never updates over the air, so every record a later build
//! writes must stay safe in the bank-0 build's hands for the life of the
//! feeder. The version says what a build may do with a record:
//!
//! - **A newer minor** of the build's major only adds fields: after the
//!   last field of every minor before it ([`Layout::fields_end`]), in room
//!   the older minor keeps zero or past its end, each reading zero as
//!   "absent". The build decodes the fields it knows and sees the rest as
//!   zero. A store that reads the record alone may grow it, its checksum
//!   at the length the header gives; one read whole ([`Extent::Unit`])
//!   keeps one length per major, since a cut erase over a longer record
//!   would leave written bytes past the build's and read as invalid, so
//!   its minors use the spare room it reserves. When the build rewrites it
//!   ([`DualSlot::replace`]), it keeps the bytes it doesn't know, and the
//!   record's minor and length with them, so the later build finds its
//!   fields again. So a minor's field must be safe for every older build
//!   both to ignore and to carry: across that build's own writes, its
//!   factory reset and its start-overs, which keep the bytes as they are.
//!   A field whose meaning such a rewrite would break (a per-client flag
//!   that would outlive the client it named, a value tied to one record
//!   that an older build copies onto the next) is not a minor's to add
//!   unless it names what it belongs to, so a reader can tell it stale.
//!   Nor is anything bank 0 must enforce: bank 0 never reads a minor's
//!   fields, so a limit kept only there binds nothing.
//! - **An older minor** of the build's major, the same length or (read
//!   alone) shorter, is read with the fields it lacks as zero, and
//!   rewritten in the build's own minor.
//! - **A newer major** is [`Found::Future`]: the store opens read-only.
//! - **An older major** the build still reads ([`Layout::oldest_major`])
//!   goes to the decoder with its version, so a store can keep one previous
//!   decoder for a major it moved on from. Anything older is
//!   [`Found::Older`].
//!
//! A change that is not additive is a new major, which bank 0 reads as a
//! newer one and stops writing: so a store bank 0 has to keep using gets a
//! new major only as a new store, in new sectors.
//!
//! # The torn-write rule
//!
//! A save writes the unit that does not hold the newest record: it erases
//! that unit, checks the whole unit erased, programs everything but the
//! marker, checks it, programs the marker, and checks the whole record.
//! A record counts only once its marker is down and its checksum holds, so
//! a power cut at any point leaves the other unit untouched and this one in
//! one of the states [`Found`] names: erased, cut short
//! ([`Found::Partial`]), unmarked or torn by a cut erase, or the new
//! record. A program clears bits only, but the part programs a whole page
//! at once and promises no byte order, so a cut program leaves any subset
//! of the bits the record clears, the marker still erased; and an erase
//! raises bits in no particular order (and a part may program bits to 0
//! inside an erase first), so a cut erase leaves anything at all, which
//! fails the checksum or loses the marker. A cut save can leave as little
//! as one cleared bit anywhere in its record, so bytes another writer left
//! that clear no bit the magic or the marker keeps set read as one too:
//! beside an erased unit, the store starts empty and writes over them.
//! That is deliberate: such bytes cannot be told from a first save cut
//! short, and taking them for another writer's would leave every store
//! whose first save was cut refusing writes for good.
//! The slot layer never retries a failed step; the store records the
//! fault, and the journal and the newest-wins stores write nothing more
//! until they are opened again (settings writes its next change to the
//! same unit, which the rule makes as safe as the first try).
//!
//! What a state means is the store's policy, since what may be lost
//! differs:
//!
//! - **Newest valid wins** ([`newest`]): the seed, Wi-Fi, client and update
//!   stores own their sectors outright, so anything but a valid record is a
//!   save of theirs cut short, and losing the newest change is safe. A
//!   newer major anywhere makes the store read-only. With no valid record,
//!   the client and update stores start empty only when nothing was ever
//!   committed ([`unwritten`]); a record of theirs they cannot read stays
//!   untouched until an explicit reset.
//! - **Settings** also falls back to the older record, reporting it
//!   recovered unless the other unit is erased. With no valid record it
//!   starts empty only after a cut first save of its own (`Partial` beside
//!   `Erased`), or when each unit is erased or holds an older major of its
//!   record (`Older`, superseded and never migrated); anything else is
//!   corrupt, and a newer major read-only, until an explicit reset.
//! - **The feed journal** may never forget a reservation. Two valid records
//!   must be consecutive; one valid record stands beside an erased unit, or
//!   beside a cut write (`Partial`, `Unmarked` or `Torn`): a pending
//!   reservation is resolved again, and beside a record at rest the cut
//!   write is the next reservation, resolved as uncertain under the next
//!   ID. It starts empty when both units are erased, or after its first
//!   record cut short beside an erased unit, when the unit holds only bits
//!   that record clears. Anything else, including a newer or older major, is
//!   corrupt.
//!
//! The journal and settings read the whole unit ([`Extent::Unit`]): written
//! bytes after the record make the unit invalid, so unrelated data is never
//! mistaken for an empty store. The others read the record alone.

use core::cmp::Ordering;
use core::ffi::c_uint;

/// The record most stores keep, and where its checksum sits.
pub const RECORD_BYTES: usize = 128;
/// CRC-32 of bytes `..CRC_AT` in a [`RECORD_BYTES`] record.
pub const CRC_AT: usize = 120;

const COMMIT: [u8; 4] = *b"CMIT";
/// The header every record shares after its 8-byte magic: the format
/// major and minor, then the record length (u16).
const MAJOR_AT: usize = 8;
const MINOR_AT: usize = 9;
const LENGTH_AT: usize = 10;
/// Where the store's fields start.
const FIELDS_AT: usize = 12;
/// How much of a unit's erased tail a scan reads at once.
const TAIL_CHUNK: u32 = 64;

const MARKER_BYTES: usize = COMMIT.len();
/// The checksum and the marker at a record's end.
const TRAILER_BYTES: usize = 4 + MARKER_BYTES;

const _: () = assert!(RECORD_BYTES == CRC_AT + TRAILER_BYTES);

/// Two or more EXCLUSIVE, nonoverlapping erase units, assigned only after a
/// complete physical flash map/backup. This interface contains no physical
/// addresses. Reads must bypass/invalidate stale caches; writes return only
/// after durable completion. Program permits aligned 4-byte granules, only
/// 1->0, splitting physical pages as needed. Erase affects exactly one whole
/// unit. All methods must be bounded. False may mean partial OR completed
/// I/O; never retry blindly. The backend must independently enforce output
/// inactivity and SDK flash/IRQ synchronization.
pub trait SlotIo {
    /// The size of one erase unit, in bytes.
    fn unit_bytes(&self) -> u32;
    fn read(&self, slot: u32, offset: u32, bytes: &mut [u8]) -> bool;
    fn program(&self, slot: u32, offset: u32, bytes: &[u8]) -> bool;
    fn erase(&self, slot: u32) -> bool;

    /// False for I/O that only reads (mapped flash with no lease): a store
    /// refuses its writes up front rather than failing part-way.
    fn writable(&self) -> bool {
        true
    }

    /// The I/O the slot layer's checks and writes run on. Code generic
    /// over the I/O is one copy per type, and every store's units are a
    /// type of their own; I/O that differs only in where its units sit
    /// returns one shared type here (the SDK's regions return their sectors
    /// as values), so that code exists once. By default, the I/O itself.
    fn shared(&self) -> impl SlotIo + '_ {
        self
    }
}

impl<T: SlotIo + ?Sized> SlotIo for &T {
    fn unit_bytes(&self) -> u32 {
        (**self).unit_bytes()
    }

    fn read(&self, slot: u32, offset: u32, bytes: &mut [u8]) -> bool {
        (**self).read(slot, offset, bytes)
    }

    fn program(&self, slot: u32, offset: u32, bytes: &[u8]) -> bool {
        (**self).program(slot, offset, bytes)
    }

    fn erase(&self, slot: u32) -> bool {
        (**self).erase(slot)
    }

    fn writable(&self) -> bool {
        (**self).writable()
    }

    fn shared(&self) -> impl SlotIo + '_ {
        (**self).shared()
    }
}

/// How much of a unit a scan reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// The record alone.
    Record,
    /// The record and every byte after it, which must be erased.
    Unit,
}

/// A record's format version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// A change an older build of the same major cannot read safely.
    pub major: u8,
    /// An additive change within the major.
    pub minor: u8,
}

impl Version {
    pub const fn new(major: u8, minor: u8) -> Self {
        Self { major, minor }
    }
}

/// What a store's record shares with every other's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub magic: [u8; 8],
    /// The version this build writes.
    pub version: Version,
    /// The oldest major this build's decoder takes; below it a record is
    /// [`Found::Older`]. The store's own major unless the store keeps a
    /// previous decoder.
    pub oldest_major: u8,
    /// Where the fields this build knows end: from here to the checksum,
    /// a record of the build's minor or older is zero, and a newer minor's
    /// bytes are kept but never decoded. A multiple of 4, from 12 to the
    /// record's checksum offset.
    pub fields_end: usize,
    pub extent: Extent,
}

impl Layout {
    /// A layout reading only its own major; `fields_end` is checked when
    /// the layout is a constant.
    pub const fn new(magic: [u8; 8], version: Version, fields_end: usize, extent: Extent) -> Self {
        assert!(fields_end >= FIELDS_AT && fields_end.is_multiple_of(4));
        Self {
            magic,
            version,
            oldest_major: version.major,
            fields_end,
            extent,
        }
    }

    /// A new record: zero, with the header filled in.
    pub fn record<const N: usize>(&self) -> [u8; N] {
        const { assert!(N >= FIELDS_AT + TRAILER_BYTES && N <= 0xffff) };
        let mut p = [0u8; N];
        if let Some(magic) = p.first_chunk_mut::<8>() {
            *magic = self.magic;
        }
        p[MAJOR_AT] = self.version.major;
        p[MINOR_AT] = self.version.minor;
        put_u16::<LENGTH_AT, N>(&mut p, u16::try_from(N).unwrap_or(u16::MAX));
        p
    }

    /// Whether the known fields end at or before the checksum of a record
    /// of `record_bytes`: each store checks its own layout against its
    /// record size as a constant, since the layout does not carry it.
    pub const fn fits(&self, record_bytes: usize) -> bool {
        record_bytes >= FIELDS_AT + TRAILER_BYTES && self.fields_end <= record_bytes - TRAILER_BYTES
    }

    /// The end of the known fields in a record of `n` bytes.
    fn known(&self, n: usize) -> usize {
        self.fields_end.min(n.saturating_sub(TRAILER_BYTES))
    }
}

/// Fill in the checksum and the marker of a finished record.
pub fn seal<const N: usize>(p: &mut [u8; N]) {
    let Some((body, trailer)) = p.split_last_chunk_mut::<TRAILER_BYTES>() else {
        return;
    };
    let crc = crc32(body);
    trailer[..4].copy_from_slice(&crc.to_le_bytes());
    trailer[4..].copy_from_slice(&COMMIT);
}

/// What one unit holds, by the torn-write rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found<V> {
    /// Every byte erased: never written, or an erase that finished.
    Erased,
    /// Committed, checked and decoded.
    Valid(V),
    /// Committed and checked, in a newer major than this build's.
    Future,
    /// Committed and checked, in a major older than any this build
    /// decodes: its own record, which no decoder here reads. Settings takes
    /// it as superseded; the client and update stores never write over it.
    Older,
    /// No marker, and the bytes could be this store's own record cut
    /// short: its magic and marker fields keep every bit they keep set.
    Partial,
    /// No marker otherwise: an erase cut short over a record, or bytes
    /// another writer left.
    Unmarked,
    /// The marker, but a failing checksum: an erase cut short before it
    /// raised the marker's bits, or a record that decayed.
    Torn,
    /// Checked but not this store's record, or written bytes after the
    /// record in a unit read whole.
    Invalid,
}

impl<V> Found<V> {
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }

    pub const fn is_erased(&self) -> bool {
        matches!(self, Self::Erased)
    }

    pub const fn is_future(&self) -> bool {
        matches!(self, Self::Future)
    }
}

/// Which of two units a store opens on, by the newest-valid-wins rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick<V> {
    /// A newer format version is present: the store is read-only.
    Future,
    /// The newer valid record, ties to unit 0, and its unit.
    Newest(c_uint, V),
    /// No valid record in either unit.
    Nothing,
}

/// The newest valid record of `a` (unit 0) and `b` (unit 1) by
/// `generation`, unless either holds a newer format version.
pub fn newest<V>(a: Found<V>, b: Found<V>, generation: impl Fn(&V) -> u64) -> Pick<V> {
    if a.is_future() || b.is_future() {
        return Pick::Future;
    }
    match (a, b) {
        (Found::Valid(x), Found::Valid(y)) => {
            if generation(&x) >= generation(&y) {
                Pick::Newest(0, x)
            } else {
                Pick::Newest(1, y)
            }
        }
        (Found::Valid(x), _) => Pick::Newest(0, x),
        (_, Found::Valid(y)) => Pick::Newest(1, y),
        _ => Pick::Nothing,
    }
}

/// Whether units `a` and `b` hold nothing a store ever committed: both
/// erased, or a first save cut short beside an erased unit, whatever
/// subset of its bits the cut left ([`Found::Partial`]). A store that
/// never writes over a record it cannot read starts empty only then. A
/// committed record of its own reads the same once its marker decays out
/// of `CMIT` while keeping every bit `CMIT` keeps set; beside an erased
/// unit (the next save's erase cut late) the store then starts empty too.
/// Bytes another writer left beside an erased unit, clearing no bit the
/// magic or the marker keeps set, read as unwritten too. That is
/// deliberate: they cannot be told from a first save cut short, and a
/// store refusing writes for good is the worse failure.
pub const fn unwritten<V>(a: &Found<V>, b: &Found<V>) -> bool {
    matches!(
        (a, b),
        (Found::Erased, Found::Erased)
            | (Found::Partial, Found::Erased)
            | (Found::Erased, Found::Partial)
    )
}

/// The unit a save writes: the first after an empty start, then the one
/// not holding the newest record.
pub const fn target(generation: u64, slot: c_uint) -> c_uint {
    if generation == 0 { 0 } else { 1 - slot }
}

/// Whether a save erases the unit first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prepare {
    Erase,
    /// The unit was just checked erased, by a first provisioning.
    Erased,
}

/// A store's two units and its record layout; before [`DualSlot::bind`],
/// every I/O call is refused.
#[derive(Clone, Copy, Debug)]
pub struct DualSlot<I> {
    io: Option<I>,
    layout: &'static Layout,
}

impl<I> DualSlot<I> {
    pub const fn unbound(layout: &'static Layout) -> Self {
        Self { io: None, layout }
    }

    pub const fn new(io: I, layout: &'static Layout) -> Self {
        Self {
            io: Some(io),
            layout,
        }
    }

    pub fn bind(&mut self, io: I) {
        self.io = Some(io);
    }

    /// The I/O it is bound to, if any.
    pub const fn io(&self) -> Option<&I> {
        self.io.as_ref()
    }
}

impl<I: SlotIo> DualSlot<I> {
    pub fn unit_bytes(&self) -> u32 {
        self.io.as_ref().map_or(0, SlotIo::unit_bytes)
    }

    /// Whether the I/O can write at all.
    pub fn writable(&self) -> bool {
        self.io.as_ref().is_some_and(SlotIo::writable)
    }

    /// The units on their shared I/O ([`SlotIo::shared`]); `None` unbound.
    fn units(&self) -> Option<Units<'_, impl SlotIo + '_>> {
        self.io.as_ref().map(|io| Units {
            io: io.shared(),
            layout: self.layout,
        })
    }

    /// Read unit `slot` by the torn-write rule, `decode` taking a record
    /// whose header, version and checksum hold, with its version. The
    /// decoder sees the build's record size: a shorter record (an older
    /// minor or major) is zero past its fields, and a newer minor zero from
    /// [`Layout::fields_end`] on. `None` when a read fails.
    pub fn scan<const N: usize, V>(
        &self,
        slot: c_uint,
        decode: impl FnOnce(Version, &[u8; N]) -> Option<V>,
    ) -> Option<Found<V>> {
        let mut p = [0u8; N];
        Some(match self.units()?.classify(slot, &mut p)? {
            Class::Current(version) => decode(version, &p).map_or(Found::Invalid, Found::Valid),
            Class::Erased => Found::Erased,
            Class::Future => Found::Future,
            Class::Older => Found::Older,
            Class::Partial => Found::Partial,
            Class::Unmarked => Found::Unmarked,
            Class::Torn => Found::Torn,
            Class::Invalid => Found::Invalid,
        })
    }

    /// Write `record` to unit `slot` by the torn-write rule. False when any
    /// step failed; the unit may then hold any state a cut leaves.
    pub fn write<const N: usize>(&self, slot: c_uint, record: &[u8; N], prepare: Prepare) -> bool {
        self.units()
            .is_some_and(|units| units.write(slot, record, prepare))
    }

    /// Write `record` to unit `slot` in place of the record in unit `from`
    /// (none on a store's first save), erasing `slot` first, by the
    /// torn-write rule. When `from` holds a newer minor of this store's
    /// major, the written record keeps that minor, its length and every
    /// byte from [`Layout::fields_end`] to its checksum, copied from
    /// `from`, so a later build finds its fields again; otherwise this is
    /// [`Self::write`]. False when any step failed, as for `write`.
    pub fn replace<const N: usize>(
        &self,
        slot: c_uint,
        from: Option<c_uint>,
        record: &[u8; N],
    ) -> bool {
        let Some(units) = self.units() else {
            return false;
        };
        let newer = match from {
            Some(from) if from != slot => match units.newer_minor(from, N) {
                Some(newer) => newer,
                None => return false,
            },
            _ => None,
        };
        match newer {
            Some(newer) => units.write_carried(slot, record, newer),
            None => units.write(slot, record, Prepare::Erase),
        }
    }

    /// Erase both units and check them erased: a store's explicit reset.
    pub fn clear(&self) -> bool {
        self.units().is_some_and(|units| units.clear())
    }
}

/// A store's units on their shared I/O, and its layout: the code here is
/// generic over the shared I/O alone, so stores whose units differ only in
/// where they sit share one copy of it.
struct Units<'a, C> {
    io: C,
    layout: &'a Layout,
}

impl<C: SlotIo> Units<'_, C> {
    fn unit_bytes(&self) -> u32 {
        self.io.unit_bytes()
    }

    fn read(&self, slot: c_uint, offset: u32, bytes: &mut [u8]) -> bool {
        self.io.read(slot, offset, bytes)
    }

    fn program(&self, slot: c_uint, offset: u32, bytes: &[u8]) -> bool {
        self.io.program(slot, offset, bytes)
    }

    fn erase(&self, slot: c_uint) -> bool {
        self.io.erase(slot)
    }

    /// The record's state, before its fields are decoded. A record to
    /// decode is left in `p` as [`Self::scan`] hands it over.
    fn classify(&self, slot: c_uint, p: &mut [u8]) -> Option<Class> {
        if !self.read(slot, 0, p) {
            return None;
        }
        let n = p.len();

        // A record of this store giving another length is read at that
        // length when it commits there; anything else at the build's own,
        // so a cut or foreign write reads as it always has.
        let length = match self.recorded_length(p) {
            Some(length) if length != n && self.commits(slot, p, length)? => length,
            _ => n,
        };
        let tail_erased = match self.layout.extent {
            Extent::Record => true,
            Extent::Unit => {
                let rest = self.tail_erased(slot, length.max(n))?;
                rest && p.get(length..).is_none_or(erased)
            }
        };

        if !tail_erased {
            return Some(Class::Invalid);
        }
        if erased(p) {
            return Some(Class::Erased);
        }
        if length == n {
            let Some((body, trailer)) = p.split_last_chunk::<TRAILER_BYTES>() else {
                return Some(Class::Invalid);
            };
            if trailer[4..] != COMMIT {
                return Some(if ours(p, &self.layout.magic) {
                    Class::Partial
                } else {
                    Class::Unmarked
                });
            }
            if trailer[..4] != crc32(body).to_le_bytes() {
                return Some(Class::Torn);
            }
        }

        let header = p.first_chunk::<FIELDS_AT>()?;
        let recorded = u16::from_le_bytes([header[LENGTH_AT], header[LENGTH_AT + 1]]);
        if header[..8] != self.layout.magic[..] || usize::from(recorded) != length {
            return Some(Class::Invalid);
        }
        let version = Version::new(header[MAJOR_AT], header[MINOR_AT]);
        // A unit read whole keeps one length within a major, as a cut erase
        // over a longer record would leave written bytes past the build's:
        // another length is only a newer major's to give.
        if self.layout.extent == Extent::Unit
            && length != n
            && version.major <= self.layout.version.major
        {
            return Some(Class::Invalid);
        }
        Some(self.admit(version, length, p))
    }

    /// Where a committed record of `length` bytes stands against this
    /// build's versions; one to decode is left in `p` ready for it.
    fn admit(&self, version: Version, length: usize, p: &mut [u8]) -> Class {
        let own = self.layout.version;
        if version.major > own.major {
            return Class::Future;
        }
        if version.major < self.layout.oldest_major {
            return Class::Older;
        }

        // A newer minor may grow the record; nothing else may outgrow the
        // build's, and the build's own minor is its size exactly.
        let n = p.len();
        let fits = if version.major < own.major {
            length <= n
        } else {
            match version.minor.cmp(&own.minor) {
                Ordering::Greater => length >= n,
                Ordering::Equal => length == n,
                Ordering::Less => length <= n,
            }
        };
        if !fits {
            return Class::Invalid;
        }

        // The decoder sees no trailer and nothing past the record, and of a
        // newer minor nothing it doesn't know.
        if let Some(past) = p.get_mut(length.min(n).saturating_sub(TRAILER_BYTES)..) {
            past.fill(0);
        }
        if version.major == own.major {
            let Some(unknown) = p.get_mut(self.layout.known(n)..) else {
                return Class::Invalid;
            };
            if version.minor > own.minor {
                unknown.fill(0);
            } else if !unknown.iter().all(|&b| b == 0) {
                return Class::Invalid;
            }
        }
        Class::Current(version)
    }

    /// The length a record of this store's magic gives, when it could be
    /// one: whole words, a header and a trailer, within the unit.
    fn recorded_length(&self, p: &[u8]) -> Option<usize> {
        let header = p.first_chunk::<FIELDS_AT>()?;
        if header[..8] != self.layout.magic[..] {
            return None;
        }
        let length = usize::from(u16::from_le_bytes([
            header[LENGTH_AT],
            header[LENGTH_AT + 1],
        ]));
        let unit = usize::try_from(self.unit_bytes()).ok()?;
        (length >= FIELDS_AT + TRAILER_BYTES && length.is_multiple_of(4) && length <= unit)
            .then_some(length)
    }

    /// Whether unit `slot` holds a record committed at `length`: the
    /// marker at its end and a checksum that holds. `p` holds the unit's
    /// first bytes. `None` when a read fails.
    fn commits(&self, slot: c_uint, p: &[u8], length: usize) -> Option<bool> {
        let end = length.checked_sub(TRAILER_BYTES)?;
        let mut trailer = [0u8; TRAILER_BYTES];
        if !self.read(slot, u32::try_from(end).ok()?, &mut trailer) {
            return None;
        }
        if trailer[4..] != COMMIT {
            return Some(false);
        }
        let crc = self.crc(slot, p, end)?;
        Some(trailer[..4] == crc.to_le_bytes())
    }

    /// CRC-32 of unit `slot`'s bytes `..end`, the first of them already
    /// read into `p`. `None` when a read fails.
    fn crc(&self, slot: c_uint, p: &[u8], end: usize) -> Option<u32> {
        let mut crc = Crc32::new();
        let held = end.min(p.len());
        crc.update(p.get(..held)?);
        let mut chunk = [0u8; TAIL_CHUNK as usize];
        let mut offset = held;
        while offset < end {
            let part = chunk.get_mut(..(end - offset).min(TAIL_CHUNK as usize))?;
            if !self.read(slot, u32::try_from(offset).ok()?, part) {
                return None;
            }
            crc.update(part);
            offset += part.len();
        }
        Some(crc.finish())
    }

    /// Whether every byte of unit `slot` from `from` on is erased; `None`
    /// when a read fails. Every chunk is read, so a failing read is
    /// reported even past written bytes.
    fn tail_erased(&self, slot: c_uint, from: usize) -> Option<bool> {
        let unit = self.unit_bytes();
        let mut offset = u32::try_from(from).ok()?;
        let mut chunk = [0u8; TAIL_CHUNK as usize];
        let mut all = true;
        while offset < unit {
            let size = (unit - offset).min(TAIL_CHUNK) as usize;
            let part = chunk.get_mut(..size)?;
            if !self.read(slot, offset, part) {
                return None;
            }
            all &= erased(part);
            offset = offset.saturating_add(TAIL_CHUNK);
        }
        Some(all)
    }

    /// Whether unit `slot` is erased throughout, read into `p` and after.
    fn unit_erased(&self, slot: c_uint, p: &mut [u8]) -> bool {
        self.read(slot, 0, p) && erased(p) && self.tail_erased(slot, p.len()) == Some(true)
    }

    /// [`DualSlot::write`].
    fn write<const N: usize>(&self, slot: c_uint, record: &[u8; N], prepare: Prepare) -> bool {
        let mut checked = [0u8; N];
        let Some((body, marker)) = record.split_last_chunk::<MARKER_BYTES>() else {
            return false;
        };
        let Ok(marker_at) = u32::try_from(body.len()) else {
            return false;
        };
        (prepare == Prepare::Erased || self.erase(slot))
            && self.unit_erased(slot, &mut checked)
            && self.program(slot, 0, body)
            && self.read(slot, 0, &mut checked)
            && checked.split_last_chunk::<MARKER_BYTES>() == Some((body, &[u8::MAX; MARKER_BYTES]))
            && self.program(slot, marker_at, marker)
            && self.read(slot, 0, &mut checked)
            && checked == *record
    }

    /// The committed newer minor of this store's major unit `from` holds,
    /// if it holds one; `None` when a read fails.
    fn newer_minor(&self, from: c_uint, n: usize) -> Option<Option<Newer>> {
        let mut header = [0u8; FIELDS_AT];
        if !self.read(from, 0, &mut header) {
            return None;
        }
        let own = self.layout.version;
        let Some(length) = self.recorded_length(&header) else {
            return Some(None);
        };
        let fits = match self.layout.extent {
            Extent::Record => length >= n,
            Extent::Unit => length == n,
        };
        if header[MAJOR_AT] != own.major || header[MINOR_AT] <= own.minor || !fits {
            return Some(None);
        }
        let commits = self.commits(from, &header, length)?;
        Some(commits.then_some(Newer {
            from,
            minor: header[MINOR_AT],
            length,
        }))
    }

    /// [`DualSlot::replace`] keeping a newer minor's bytes: the body a chunk at
    /// a time, in order, then the checksum, a check, the marker and a check.
    /// A cut leaves the chunks before it, then some of the cut chunk's bits.
    fn write_carried(&self, slot: c_uint, record: &[u8], newer: Newer) -> bool {
        let Some(end) = newer.length.checked_sub(TRAILER_BYTES) else {
            return false;
        };
        let Ok(end_at) = u32::try_from(end) else {
            return false;
        };
        let mut chunk = [0u8; TAIL_CHUNK as usize];
        if !(self.erase(slot) && self.unit_erased(slot, &mut chunk)) {
            return false;
        }

        let mut crc = Crc32::new();
        let mut offset = 0;
        while offset < end {
            let Some(part) = chunk.get_mut(..(end - offset).min(TAIL_CHUNK as usize)) else {
                return false;
            };
            let programmed = self.carried(record, newer, offset, part)
                && u32::try_from(offset).is_ok_and(|at| self.program(slot, at, part));
            if !programmed {
                return false;
            }
            crc.update(part);
            offset += part.len();
        }

        let crc = crc.finish().to_le_bytes();
        self.program(slot, end_at, &crc)
            && self.carried_holds(slot, record, newer, crc, [u8::MAX; MARKER_BYTES])
            && self.program(slot, end_at + 4, &COMMIT)
            && self.carried_holds(slot, record, newer, crc, COMMIT)
    }

    /// Bytes `offset..` of the carried record into `out`: the header with
    /// the newer minor and length, the build's fields, then `from`'s bytes.
    fn carried(&self, record: &[u8], newer: Newer, offset: usize, out: &mut [u8]) -> bool {
        let own = self
            .layout
            .known(record.len())
            .saturating_sub(offset)
            .min(out.len());
        let (head, rest) = out.split_at_mut(own);
        if own > 0 {
            let Some(fields) = record.get(offset..offset + own) else {
                return false;
            };
            head.copy_from_slice(fields);
        }
        let kept = rest.is_empty()
            || u32::try_from(offset + own).is_ok_and(|at| self.read(newer.from, at, rest));
        if !kept {
            return false;
        }

        let length = u16::try_from(newer.length)
            .unwrap_or(u16::MAX)
            .to_le_bytes();
        for (at, byte) in [
            (MINOR_AT, newer.minor),
            (LENGTH_AT, length[0]),
            (LENGTH_AT + 1, length[1]),
        ] {
            if let Some(out) = at.checked_sub(offset).and_then(|i| out.get_mut(i)) {
                *out = byte;
            }
        }
        true
    }

    /// Whether unit `slot` holds the carried record as written, then `crc`
    /// and `marker`.
    fn carried_holds(
        &self,
        slot: c_uint,
        record: &[u8],
        newer: Newer,
        crc: [u8; 4],
        marker: [u8; MARKER_BYTES],
    ) -> bool {
        let Some(end) = newer.length.checked_sub(TRAILER_BYTES) else {
            return false;
        };
        let mut want = [0u8; TAIL_CHUNK as usize];
        let mut got = [0u8; TAIL_CHUNK as usize];
        let mut offset = 0;
        while offset < end {
            let size = (end - offset).min(TAIL_CHUNK as usize);
            let (Some(w), Some(g)) = (want.get_mut(..size), got.get_mut(..size)) else {
                return false;
            };
            let holds = self.carried(record, newer, offset, w)
                && u32::try_from(offset).is_ok_and(|at| self.read(slot, at, g))
                && w == g;
            if !holds {
                return false;
            }
            offset += size;
        }

        let mut trailer = [0u8; TRAILER_BYTES];
        u32::try_from(end).is_ok_and(|at| self.read(slot, at, &mut trailer))
            && trailer[..4] == crc
            && trailer[4..] == marker
    }

    /// [`DualSlot::clear`].
    fn clear(&self) -> bool {
        let mut p = [0u8; TAIL_CHUNK as usize];
        self.erase(0) && self.erase(1) && self.unit_erased(0, &mut p) && self.unit_erased(1, &mut p)
    }
}

/// A newer minor [`DualSlot::replace`] carries: its unit, minor and length.
#[derive(Clone, Copy)]
struct Newer {
    from: c_uint,
    minor: u8,
    length: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Erased,
    /// Header, version and checksum hold: the store decodes the fields.
    Current(Version),
    Future,
    Older,
    Partial,
    Unmarked,
    Torn,
    Invalid,
}

fn erased(p: &[u8]) -> bool {
    p.iter().all(|&b| b == u8::MAX)
}

/// Whether the unmarked `p` could be this store's own save cut short: its
/// magic and its marker, the two fields every record of the store shares,
/// keep every bit they keep set. The part programs a whole page at once
/// with no byte order and the marker only after the rest, so a cut save
/// leaves any subset of the bits its record clears, and these two fields
/// show it; a cut erase over one, as this model assumes, only raises bits
/// and keeps it so. Bytes another writer left clear a bit of the magic or
/// the marker that the store keeps set, unless they read nearly erased.
fn ours(p: &[u8], magic: &[u8; 8]) -> bool {
    let kept = |bytes: Option<&[u8]>, field: &[u8]| {
        bytes.is_some_and(|bytes| bytes.iter().zip(field).all(|(&b, &f)| b & f == f))
    };
    let marker_at = p.len().saturating_sub(MARKER_BYTES);
    p.len() >= FIELDS_AT + TRAILER_BYTES
        && kept(p.get(..magic.len()), magic)
        && kept(p.get(marker_at..), &COMMIT)
}

// Little-endian record fields at a fixed offset `AT` in a fixed-size record.
// An offset past the record's end fails to compile, so no input can make
// these panic.

const fn fits<const AT: usize, const N: usize, const L: usize>() {
    assert!(AT + N <= L, "record field past the record's end");
}

fn put<const AT: usize, const N: usize, const L: usize>(p: &mut [u8; L], bytes: [u8; N]) {
    const { fits::<AT, N, L>() };
    if let Some(field) = p.get_mut(AT..).and_then(<[u8]>::first_chunk_mut) {
        *field = bytes;
    }
}

fn get<const AT: usize, const N: usize, const L: usize>(p: &[u8; L]) -> [u8; N] {
    const { fits::<AT, N, L>() };
    p.get(AT..)
        .and_then(<[u8]>::first_chunk)
        .copied()
        .unwrap_or([0; N])
}

pub fn put_u16<const AT: usize, const L: usize>(p: &mut [u8; L], value: u16) {
    put::<AT, 2, L>(p, value.to_le_bytes());
}

pub fn put_u32<const AT: usize, const L: usize>(p: &mut [u8; L], value: u32) {
    put::<AT, 4, L>(p, value.to_le_bytes());
}

pub fn put_u64<const AT: usize, const L: usize>(p: &mut [u8; L], value: u64) {
    put::<AT, 8, L>(p, value.to_le_bytes());
}

pub fn get_u16<const AT: usize, const L: usize>(p: &[u8; L]) -> u16 {
    u16::from_le_bytes(get::<AT, 2, L>(p))
}

pub fn get_u32<const AT: usize, const L: usize>(p: &[u8; L]) -> u32 {
    u32::from_le_bytes(get::<AT, 4, L>(p))
}

pub fn get_u64<const AT: usize, const L: usize>(p: &[u8; L]) -> u64 {
    u64::from_le_bytes(get::<AT, 8, L>(p))
}

/// CRC-32/ISO-HDLC: reflected 0x04c11db7, init/xor-out all ones.
pub fn crc32(p: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(p);
    crc.finish()
}

/// [`crc32`] over bytes that arrive in pieces, such as a record read from
/// flash a chunk at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub const fn new() -> Self {
        Self(u32::MAX)
    }

    pub fn update(&mut self, p: &[u8]) {
        for &byte in p {
            self.0 ^= u32::from(byte);
            for _ in 0..8 {
                self.0 = (self.0 >> 1) ^ if self.0 & 1 != 0 { 0xedb8_8320 } else { 0 };
            }
        }
    }

    pub const fn finish(self) -> u32 {
        !self.0
    }
}
