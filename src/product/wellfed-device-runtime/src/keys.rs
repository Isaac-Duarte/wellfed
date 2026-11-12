//! The client store's changes and the feeder's identity, carried
//! between the loop task and the settings writer.
//!
//! The settings writer alone writes the client store (storage sectors 9 and
//! 10), and the loop task alone runs handshakes and reads the panel. So the
//! loop queues a change here with [`request`] (an enrollment once a
//! handshake admits a client, a revocation, or the reset), and the
//! writer takes it with [`take`] and frees the slot with [`finish`],
//! naming the [`KeyResult`]. A second change before the writer takes the
//! first is [`Requested::Busy`], never queued behind it. A caller that must
//! know the result (BLE setup answers its client) keeps the [`ticket`] of
//! its change and reads [`outcome`]. The mailbox remembers the last
//! [`KEYS_FINISHED`] results, so a ticket stays readable after newer
//! changes finish behind it.
//!
//! The history does not wait for anyone to read the result: the writer
//! appends an enrollment's or revocation's event in the turn that makes
//! the change (`writer::Core::change`), so a waiter that has gone, or a
//! restart before any pass reads the ring of results, costs no event.
//!
//! The other way, the writer publishes the identity secret it holds with
//! [`publish_identity`], each time with a new generation, and the loop
//! copies it with [`identity_since`] to derive the public key. The secret
//! stays in RAM: no log, request or record carries it. The writer also
//! publishes the enrolled clients ([`publish_clients`]), which the device
//! API reads to admit a session.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use wellfed_device_api::clients::{CLIENTS as STORE_CLIENTS, Client, Role};
pub use wellfed_device_api::mailbox::{
    ClientChange as Change, KEYS_FINISHED, KeyResult, Requested,
};

use wellfed_hal::cell::TaskCell;
use wellfed_hal::handoff::{Handoff, Taken};
use wellfed_hal::ota::signers;
use wellfed_hal::sdk;

// Every update key the store can hold reaches the update stage.
const _: () = assert!(STORE_CLIENTS <= signers::SIGNERS);

/// The queued change, from the loop to the writer.
static PENDING: Handoff<Change> = Handoff::new(Change::Reset);
/// The latest queued change's ticket; only the claimant writes it.
static TICKET: AtomicU32 = AtomicU32::new(0);
/// The last finished changes, a ring the writer fills at `NEXT`: each its
/// ticket's low 24 bits, then its result's index in `KeyResult::ALL` plus
/// one (0 for an entry never written). Only the writer stores either; the
/// loop reads each entry whole, so it sees an entry before or after its
/// overwrite, never a mix.
static FINISHED: [AtomicU32; KEYS_FINISHED] = [const { AtomicU32::new(0) }; KEYS_FINISHED];
static NEXT: AtomicU32 = AtomicU32::new(0);

/// Written by the writer and read by the loop, each with the scheduler
/// paused; `GENERATION` is 0 until the first publish.
static IDENTITY: TaskCell<[u8; 32]> = TaskCell::new([0; 32]);
static GENERATION: AtomicU32 = AtomicU32::new(0);

/// The loop's half: queue one change for the writer.
pub fn request(change: Change) -> Requested {
    // The ticket moves while the slot is claimed, so it names this change.
    let queued = PENDING.offer(|pending| {
        *pending = change;
        TICKET.store(
            TICKET.load(Ordering::Acquire).wrapping_add(1),
            Ordering::Release,
        );
    });
    if !queued {
        return Requested::Busy;
    }
    wellfed_hal::writer_wake::wake();
    Requested::Queued
}

/// Whether a change waits for the writer (queued, being written or taken).
pub fn queued() -> bool {
    PENDING.waiting()
}

/// The ticket of the change `request` last queued.
pub fn ticket() -> u32 {
    TICKET.load(Ordering::Acquire)
}

/// The result of the change with `ticket`, once the writer has finished it.
pub fn outcome(ticket: u32) -> Option<KeyResult> {
    let finished = FINISHED
        .iter()
        .map(|entry| entry.load(Ordering::Acquire))
        .find(|&finished| finished & 0xff != 0 && finished >> 8 == ticket & 0x00ff_ffff)?;
    let index = usize::try_from(finished & 0xff).ok()?.checked_sub(1)?;
    KeyResult::ALL.get(index).copied()
}

/// The writer's half: the queued change, held until [`finish`].
pub fn take() -> Option<Taken<'static, Change>> {
    PENDING.take()
}

/// The writer's half: the taken change is applied or refused; record its
/// result, then free the slot.
pub fn finish(taken: Taken<'static, Change>, result: KeyResult) {
    let index = KeyResult::ALL
        .iter()
        .position(|&r| r == result)
        .and_then(|i| u32::try_from(i + 1).ok())
        .unwrap_or(0);
    let next = NEXT.load(Ordering::Acquire);
    let slot = usize::try_from(next).unwrap_or(0) % KEYS_FINISHED;
    FINISHED[slot].store(
        (TICKET.load(Ordering::Acquire) & 0x00ff_ffff) << 8 | index,
        Ordering::Release,
    );
    NEXT.store(next.wrapping_add(1), Ordering::Release);
    taken.finish();
}

/// The writer's half: the identity the store now holds.
pub fn publish_identity(secret: &[u8; 32]) {
    sdk::unscheduled(|| {
        // SAFETY: with the scheduler paused no other task runs, and no
        // interrupt handler reaches the identity.
        unsafe { *IDENTITY.get() = *secret };
        let next = GENERATION.load(Ordering::Acquire).wrapping_add(1).max(1);
        GENERATION.store(next, Ordering::Release);
    });
}

/// The loop's half: the identity and its generation, if published since
/// `seen` (0 before the first).
pub fn identity_since(seen: u32) -> Option<(u32, [u8; 32])> {
    sdk::unscheduled(|| {
        let generation = GENERATION.load(Ordering::Acquire);
        // SAFETY: as in `publish_identity`.
        (generation != 0 && generation != seen).then(|| (generation, unsafe { *IDENTITY.get() }))
    })
}

/// The enrolled clients as the writer last published them: written by the
/// writer and read by the loop, each with the scheduler paused, so a reader
/// never sees half a table. `CLIENTS_GENERATION` is 0 until the first.
static CLIENTS: TaskCell<[Option<Client>; STORE_CLIENTS]> = TaskCell::new([None; STORE_CLIENTS]);
static CLIENTS_GENERATION: AtomicU32 = AtomicU32::new(0);

/// The writer's half: the clients the store now holds, after it loads and
/// after every change. Their update keys go to the update stage too.
pub fn publish_clients(clients: impl Iterator<Item = Client>) {
    let mut table = [None; STORE_CLIENTS];
    for (slot, client) in table.iter_mut().zip(clients) {
        *slot = Some(client);
    }
    signers::publish(
        table
            .iter()
            .flatten()
            .filter(|client| client.role == Role::UpdateSigner)
            .map(|client| client.key),
    );
    sdk::unscheduled(|| {
        // SAFETY: as in `publish_identity`.
        unsafe { *CLIENTS.get() = table };
        let next = CLIENTS_GENERATION
            .load(Ordering::Acquire)
            .wrapping_add(1)
            .max(1);
        CLIENTS_GENERATION.store(next, Ordering::Release);
    });
}

/// The loop's half: copy the clients into `table` if published since `seen`
/// (0 before the first), returning their generation. The device API checks
/// every session's key against its copy; copying in place keeps the half
/// kilobyte off the loop's stack.
pub fn clients_since(seen: u32, table: &mut [Option<Client>; STORE_CLIENTS]) -> Option<u32> {
    sdk::unscheduled(|| {
        let generation = CLIENTS_GENERATION.load(Ordering::Acquire);
        if generation == 0 || generation == seen {
            return None;
        }
        // SAFETY: as in `publish_identity`.
        *table = unsafe { *CLIENTS.get() };
        Some(generation)
    })
}

/// The writer could not finish a factory reset this boot and asks the loop
/// for one restart to try again.
static RESTART: AtomicBool = AtomicBool::new(false);

/// The writer's half: ask for the restart.
pub fn want_restart() {
    RESTART.store(true, Ordering::Release);
}

/// The loop's half: whether the writer asked for a restart.
pub fn restart_wanted() -> bool {
    RESTART.load(Ordering::Acquire)
}

/// Forget everything (host tests).
#[cfg(not(target_os = "none"))]
pub fn reset() {
    RESTART.store(false, Ordering::Release);
    PENDING.clear();
    TICKET.store(0, Ordering::Release);
    for entry in &FINISHED {
        entry.store(0, Ordering::Release);
    }
    NEXT.store(0, Ordering::Release);
    GENERATION.store(0, Ordering::Release);
    // SAFETY: host tests serialize on this module's state.
    unsafe { *IDENTITY.get() = [0; 32] };
    CLIENTS_GENERATION.store(0, Ordering::Release);
    // SAFETY: as above.
    unsafe { *CLIENTS.get() = [None; STORE_CLIENTS] };
}
