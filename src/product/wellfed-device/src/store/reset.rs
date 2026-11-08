//! The factory reset's marker on NOR: one sector (`storage::RESET`) that
//! says a reset was asked for and has not finished.
//!
//! A reset spans many sectors, and a power cut can fall between any two
//! erases. So the settings writer programs the marker before it changes
//! anything, the boot that finds it erases every cleared store before it
//! opens one, and the marker is erased last. A cut before the marker lands
//! leaves the feeder as it was (the owner holds Reset again); a cut after it
//! leaves a reset the next boot finishes, however far it got.
//!
//! The marker is a fixed 16-byte pattern at the sector's start. Only that
//! exact pattern counts: a torn program reads as no reset (nothing was
//! erased yet), and a torn erase of the marker at the very end either still
//! reads as the pattern, so the boot erases the cleared stores again, or as
//! no reset, which is right because they are already erased. Bits a sector
//! never written may hold are not a reset either.
//!
//! After the marker come [`ATTEMPTS`] attempt words, each programmed (no
//! erase) by an armed boot as it starts the erases. A boot's reset flags
//! cannot tell an intentional restart from a hang (both are the watchdog's
//! reset on this board), and every backup register is taken, so the count
//! lives here: it bounds the restarts a boot that cannot finish asks for.
//!
//! Before the marker, [`mark`] programs a 16-byte word saying the owner
//! asked for the reset by hand, at the panel. Only such a reset erases an
//! update store this build cannot read (`UpdateStore::clear_corrupt`):
//! whoever stands at the feeder could erase it over UART anyway, while a
//! reset asked for any other way must never reopen a downgrade. The word
//! goes first, so a marker that reads back means the word is down too, and
//! a reset some later build asks for another way leaves it out, so a bank 0
//! that finishes that reset keeps the update store.
//!
//! A mark cut after its word but before its marker leaves the word alone in
//! the sector, and the next mark keeps it rather than erase. That is safe
//! only because every mark this build makes is by hand: the stale word
//! says no more than the new one would. A later build that marks a reset
//! another way must erase a sector holding the word first, and the finished
//! reset's erase of the whole sector clears it either way.

use wellfed_core::store::slot::SlotIo;

/// The marker's bytes, at the start of its sector. Stock's handover mark
/// shares the sector, and has to know this one to leave it alone.
pub use wellfed_core::store::handover::RESET_MARKER as MARKER;

/// Whether `bytes`, read from the marker's sector, say a reset is pending.
pub fn marked(bytes: &[u8; MARKER.len()]) -> bool {
    *bytes == MARKER
}

/// Whether the marker sector `io` (one unit) holds a pending reset. A
/// sector that cannot be read holds none: the stores open as they are.
pub fn pending<I: SlotIo>(io: &I) -> bool {
    let mut bytes = [0; MARKER.len()];
    io.read(0, 0, &mut bytes) && marked(&bytes)
}

/// Program the factory reset the owner asked for by holding Reset: erase
/// the sector first unless it is blank (or already marked), then program
/// the by-hand word and the marker, and read the marker back. Every
/// factory reset today is the panel's Reset hold.
pub fn mark<I: SlotIo>(io: &I) -> bool {
    let mut bytes = [0; MARKER.len()];
    if !io.read(0, 0, &mut bytes) {
        return false;
    }
    if marked(&bytes) {
        return true;
    }

    let mut hand = [0; BY_HAND.len()];
    if !io.read(0, BY_HAND_AT, &mut hand) {
        return false;
    }
    let blank = |b: &[u8]| b.iter().all(|&byte| byte == u8::MAX);
    if !(blank(&bytes) && (blank(&hand) || hand == BY_HAND)) && !io.erase(0) {
        return false;
    }
    io.program(0, BY_HAND_AT, &BY_HAND) && io.program(0, 0, &MARKER) && pending(io)
}

/// What [`mark`] programs before the marker: the reset was asked for at
/// the panel.
pub const BY_HAND: [u8; 16] = *b"PLFRST01BYHAND!!";

/// Where the by-hand word sits: after the marker and its attempts.
const BY_HAND_AT: u32 = (ATTEMPTS + 1) * 16;

/// Whether the pending reset in the marker sector `io` was asked for by
/// hand. Only the exact word counts; an unreadable sector says no.
pub fn by_hand<I: SlotIo>(io: &I) -> bool {
    let mut bytes = [0; BY_HAND.len()];
    pending(io) && io.read(0, BY_HAND_AT, &mut bytes) && bytes == BY_HAND
}

/// Erase the marker and its attempts: the reset is over.
pub fn clear<I: SlotIo>(io: &I) -> bool {
    io.erase(0)
}

/// Armed boots that may start the reset's erases: after this many, a boot
/// that cannot finish waits for a power cycle instead of asking for a
/// restart. The first is the boot the hold restarts into; the second is the
/// one escape restart.
pub const ATTEMPTS: u32 = 2;

/// What an attempt programs, in its own 16 bytes after the marker.
pub const ATTEMPT: [u8; 16] = *b"PLFRST01ATTEMPT!";

/// The byte offset of attempt `index`.
fn attempt_at(index: u32) -> u32 {
    (index + 1) * 16
}

/// How many boots started the erases, up to [`ATTEMPTS`]. Any programmed
/// bit counts, so a torn attempt is one: it can only stop a restart, never
/// start one. Unreadable reads as all used.
pub fn attempts<I: SlotIo>(io: &I) -> u32 {
    (0..ATTEMPTS)
        .filter(|&index| {
            let mut bytes = [0; ATTEMPT.len()];
            !io.read(0, attempt_at(index), &mut bytes) || bytes.iter().any(|&b| b != u8::MAX)
        })
        .count()
        .try_into()
        .unwrap_or(ATTEMPTS)
}

/// Record that this boot starts the erases: program the next blank
/// attempt, with no erase, so it survives every reset until the marker is
/// cleared. True once the count reads back one higher, or when every
/// attempt is already used. A program that does not land is false (the
/// reset stops without asking for a restart), so flash that takes no
/// write can never restart the feeder over and over.
pub fn note_attempt<I: SlotIo>(io: &I) -> bool {
    let used = attempts(io);
    if used >= ATTEMPTS {
        return true;
    }
    io.program(0, attempt_at(used), &ATTEMPT) && attempts(io) > used
}
