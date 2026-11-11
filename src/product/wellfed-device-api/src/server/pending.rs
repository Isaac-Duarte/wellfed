//! Writes the device finishes on a later pass. A write is remembered as
//! under way from the moment it is queued (`Remembered::Waiting`); the
//! session that asked polls it each pass, then answers it and remembers the
//! answer once the device is done. A retry of it, on any session, waits for
//! that answer rather than running the write again, and a write whose
//! session has gone is settled by the pass ([`Server::settle_left`]).

use super::feeder::{self, FeederReply};
use super::handshake::find;
use super::memory::{Recall, Remembered, recall, remembered};
use super::request::{Action, Name, Reply};
use super::{Pending, Phase, Server, Wait, Waiting, slot_id};
use crate::clients::Role;
use crate::mailbox::{NtpResult, NtpServer, Origin, SaveOutcome, TimeSettingsResult, WifiResult};
use crate::pairing::{self, Pairing, Path};
use crate::{Device, KeyChange, KeyOutcome, Reason, Record, UpdateAnswer, UpdateKeyChange};
use wellfed_proto::api::{
    self as proto, DeviceMessage, Outcome, Refusal, RequestId, TimeView, UpdateRefusal,
};
use wellfed_signed::offer::key_id;

impl Server {
    /// Hold the answer until the device finishes the write: one the session
    /// waits on alone (an update key's press), or a retry of one under way.
    pub(super) fn wait(
        &mut self,
        index: usize,
        id: RequestId,
        action: Action,
        waiting: Waiting,
    ) -> Option<Reply> {
        if let Phase::Open { pending, .. } = &mut self.slots[index].phase {
            *pending = Some(Pending {
                id,
                op: action.name(),
                waiting,
            });
        }
        None
    }

    /// Hold the answer until the device finishes the write just queued, and
    /// remember the wait under its id at once: a retry, on this session or
    /// on another after a reconnect, waits for this write instead of running
    /// it again, and the pass settles it if this session goes first.
    pub(super) fn queued(
        &mut self,
        index: usize,
        id: RequestId,
        action: Action,
        wait: Wait,
    ) -> Option<Reply> {
        if let Phase::Open { memory, .. } = self.slots[index].phase {
            self.memories[memory].remember(id, Remembered::Waiting(wait));
        }
        self.wait(index, id, action, Waiting::Write(wait))
    }

    /// Settle every write whose session has gone, so its client's memory
    /// holds the answer for a retry, and holds it as the device finished it
    /// rather than as the stores read when the retry comes. A write its own
    /// session still waits on is that session's to poll. Out of line, so
    /// its polls stay out of `step`'s frame.
    #[inline(never)]
    pub(super) fn settle_left<D: Device>(
        &mut self,
        device: &mut D,
        record: &mut impl FnMut(Record),
    ) {
        for memory in 0..self.memories.len() {
            for entry in 0..crate::REMEMBERED {
                let Some((id, Remembered::Waiting(wait))) = self.memories[memory].entry(entry)
                else {
                    continue;
                };
                let asking = self.slots.iter().any(|slot| {
                    matches!(
                        slot.phase,
                        Phase::Open {
                            memory: held,
                            pending: Some(Pending {
                                id: waited,
                                waiting: Waiting::Write(_),
                                ..
                            }),
                            ..
                        } if held == memory && waited == id
                    )
                });
                if asking {
                    continue;
                }
                let settled = match poll(wait, u8::MAX, device, record) {
                    Some(Settled::Answer(reply)) => remembered(reply),
                    Some(Settled::Lost) => Remembered::Lost,
                    None => continue,
                };
                self.memories[memory].remember(id, settled);
            }
        }
    }

    /// A pending write's answer, once the device has finished it.
    pub(super) fn poll_pending<D: Device>(
        &mut self,
        index: usize,
        device: &mut D,
        pairing: &mut Pairing,
        now_ms: u32,
        record: &mut impl FnMut(Record),
    ) -> Result<Option<(RequestId, &'static str, Reply)>, Reason> {
        let Phase::Open {
            pending: Some(pending),
            memory,
            ..
        } = self.slots[index].phase
        else {
            return Ok(None);
        };

        let outcome = match pending.waiting {
            Waiting::Write(wait) => match poll(wait, slot_id(index), device, record) {
                Some(Settled::Answer(reply)) => reply,
                // Nobody can say what the write did: the session ends with
                // it unanswered, which its client holds as uncertain, and
                // a retry hears the same.
                Some(Settled::Lost) => {
                    self.memories[memory].remember(pending.id, Remembered::Lost);
                    return Err(Reason::Forgotten);
                }
                None => return Ok(None),
            },
            // The write's own session, or the pass once it has gone, settles
            // it; a retry hears what the memory holds then, as any retry
            // does. The memory keeps a write under way for as long as a
            // session of its client is open (`Memory::remember`), so this
            // session cannot lose it; a write whose outcome the device lost
            // ends it unanswered.
            Waiting::Retry => {
                let remembered = self.memories[memory]
                    .find(&pending.id)
                    .ok_or(Reason::Forgotten)?;
                let reply = match recall(remembered, device, pairing, now_ms) {
                    Recall::Answer(reply) => reply,
                    Recall::Waiting => return Ok(None),
                    Recall::Lost => return Err(Reason::Forgotten),
                };
                if let Phase::Open { pending, .. } = &mut self.slots[index].phase {
                    *pending = None;
                }
                return Ok(Some((pending.id, pending.op, reply)));
            }
            Waiting::Signer { key, label } => match self.signer {
                Some(pairing::Event::Admit {
                    path: Path::Signer, ..
                }) => {
                    let change = KeyChange::Enroll {
                        key,
                        label,
                        role: Role::UpdateSigner,
                        path: proto::Path::Press,
                    };
                    // The mailbox may be busy; the press is kept until it is not.
                    if let Some(ticket) = device.keys(change) {
                        self.signer = None;
                        self.waiting(index, Waiting::SignerKeys { ticket, key });
                    }
                    return Ok(None);
                }
                Some(pairing::Event::Refused {
                    path: Path::Signer, ..
                }) => {
                    self.signer = None;
                    Reply::Refused(Refusal::Update(UpdateRefusal::Unconfirmed))
                }
                _ => return Ok(None),
            },
            Waiting::SignerKeys { ticket, key } => match device.keys_outcome(ticket) {
                None => return Ok(None),
                // Already enrolled under another role: the key is a session
                // key, never an update signer, and stays what it was.
                Some(KeyOutcome::Duplicate)
                    if find(&self.clients, &key).is_some_and(|c| c.role != Role::UpdateSigner) =>
                {
                    Reply::Refused(Refusal::Update(UpdateRefusal::HeldAsSessionKey))
                }
                Some(KeyOutcome::Done | KeyOutcome::Duplicate) => {
                    signer_landed(slot_id(index), &key, record);
                    Reply::Done(None)
                }
                Some(KeyOutcome::Full) => Reply::Refused(Refusal::Full),
                Some(KeyOutcome::NotFound | KeyOutcome::NotWritable) => {
                    Reply::Refused(Refusal::NotWritable)
                }
                Some(KeyOutcome::Lost) => return Err(Reason::Forgotten),
            },
        };

        if let Phase::Open { pending, .. } = &mut self.slots[index].phase {
            *pending = None;
        }
        self.memories[memory].remember(pending.id, remembered(outcome));
        Ok(Some((pending.id, pending.op, outcome)))
    }

    /// Move a pending request on to its next wait.
    fn waiting(&mut self, index: usize, next: Waiting) {
        if let Phase::Open {
            pending: Some(pending),
            ..
        } = &mut self.slots[index].phase
        {
            pending.waiting = next;
        }
    }

    /// One request's line and answer.
    #[expect(clippy::too_many_arguments, reason = "one answer's whole context")]
    pub(super) fn answer<D: Device>(
        &mut self,
        index: usize,
        id: RequestId,
        op: &'static str,
        reply: Reply,
        now_ms: u32,
        device: &mut D,
        record: &mut impl FnMut(Record),
    ) -> Result<(), Reason> {
        // A chunk that went in, or a page of bank 1 read out, is not worth
        // a line each: an image is about two thousand of them. Nor is a chunk
        // refused `Busy`, which a client sends again at once, about two
        // hundred times for one meal call.
        let chunk = op == Action::CHUNK || op == Action::CALL_CHUNK;
        if !(op == Action::CHUNK && matches!(reply, Reply::Update(_)))
            && !(op == Action::CALL_CHUNK && matches!(reply, Reply::Feeder(FeederReply::Call)))
            && !(chunk && matches!(reply, Reply::Refused(Refusal::Busy)))
            && !matches!(reply, Reply::Bank1 { .. })
        {
            record(Record::Request {
                slot: slot_id(index),
                op,
                result: reply.name(),
            });
        }

        // The client list encodes itself, so neither it nor the line above
        // sits under `seal_answer`'s frame.
        match reply {
            Reply::Clients => self.list_clients(index, id),
            reply => self.seal_answer(index, id, reply, now_ms, device),
        }
    }

    /// Seal one request's answer. Kept out of line: the wire message is as
    /// large as its largest variant, so only this frame holds one.
    #[inline(never)]
    fn seal_answer<D: Device>(
        &mut self,
        index: usize,
        id: RequestId,
        reply: Reply,
        now_ms: u32,
        device: &mut D,
    ) -> Result<(), Reason> {
        // The time view's zone borrows this until the answer is sealed.
        let zone;
        let result = match reply {
            // `answer` takes this one before it gets here: a direct call to
            // `list_clients` would put this frame back under it, in the stack
            // audit's bound.
            Reply::Clients => Outcome::Refused(Refusal::Unavailable),
            Reply::Done(generation) => Outcome::Done { generation },
            Reply::Refused(refusal) => Outcome::Refused(refusal),
            Reply::Invite(invite) => Outcome::Invite(invite),
            Reply::Update(view) => Outcome::Update(view),
            Reply::Feeder(reply) => return self.seal_feeder(index, id, reply, device.feeder()),
            Reply::Status => {
                let mut status = device.status(now_ms);
                status.sessions = u8::try_from(self.sessions()).unwrap_or(u8::MAX);
                Outcome::Status(status)
            }
            Reply::Time => match time_view(device, now_ms) {
                Some((view, text)) => {
                    zone = text;
                    Outcome::Time(TimeView {
                        zone: zone.as_str(),
                        ..view
                    })
                }
                None => Outcome::Refused(Refusal::Unavailable),
            },
            Reply::Stats(section) => {
                let message = DeviceMessage::Response {
                    id,
                    result: Outcome::Stats(device.stats(section, now_ms)),
                };
                return self.seal_and_send(index, &message);
            }
            Reply::Alerts => match device.alerts() {
                Some(view) => Outcome::Alerts(view),
                None => Outcome::Refused(Refusal::Unavailable),
            },
            Reply::Bank1 { offset } => {
                let result = match device.bank1_page(offset) {
                    Ok(page) => Outcome::Bank1(page),
                    Err(refusal) => Outcome::Refused(refusal),
                };
                return self.seal_and_send(index, &DeviceMessage::Response { id, result });
            }
        };
        self.seal_and_send(index, &DeviceMessage::Response { id, result })
    }
}

/// A pressed key's enrollment went through, read by its session. Its
/// history event is the writer's, recorded with the store write whether or
/// not the session stayed to read this.
fn signer_landed(slot: u8, key: &[u8; 32], record: &mut impl FnMut(Record)) {
    record(Record::UpdateKey {
        slot,
        change: UpdateKeyChange::Enrolled,
        key: key_id(key),
    });
}

/// What became of a write the device has finished.
enum Settled {
    Answer(Reply),
    /// It finished, and its outcome is gone before anyone read it: a later
    /// request's replaced it. Nobody can say what it did.
    Lost,
}

/// A write's answer once the device has finished it; `None` while it is
/// still under way. `slot` names the session whose answer it is in the
/// log, `u8::MAX` for one that has gone.
fn poll<D: Device>(
    wait: Wait,
    slot: u8,
    device: &mut D,
    record: &mut impl FnMut(Record),
) -> Option<Settled> {
    Some(Settled::Answer(match wait {
        Wait::Feeder(wait) => match device.feeder() {
            Some(feeder) => feeder::poll(wait, feeder)?,
            None => Reply::Refused(Refusal::Unavailable),
        },
        // The writer's outcome for this change decides: a store no longer
        // pending says only that the writer finished, and a save it could
        // not make (unarmed, or refused) leaves the old credentials. It
        // recorded the `Wifi` event as it saved one.
        Wait::Wifi { change } => match device.wifi(None) {
            WifiResult::Current { view, .. } => match view.saved.outcome(change) {
                SaveOutcome::Pending => return None,
                SaveOutcome::Done => Reply::Done(Some(view.generation)),
                SaveOutcome::Failed => Reply::Refused(Refusal::NotWritable),
            },
            WifiResult::Busy => return None,
            WifiResult::ReadOnly | WifiResult::Invalid => Reply::Refused(Refusal::NotWritable),
        },
        // No event: the history's settings events carry the settings
        // store's generation, and the time settings have their own. A
        // write the writer could not make (unarmed, or a failed save)
        // leaves the store as it was: refused, never `Done`.
        Wait::Time { digest } => match device.time_settings(None) {
            TimeSettingsResult::Current {
                view,
                pending: false,
            } if view.settings.digest() == digest => Reply::Done(Some(view.generation)),
            TimeSettingsResult::Current { pending: false, .. } => {
                Reply::Refused(Refusal::NotWritable)
            }
            TimeSettingsResult::Current { pending: true, .. } | TimeSettingsResult::Busy => {
                return None;
            }
            TimeSettingsResult::ReadOnly | TimeSettingsResult::Unserved => {
                Reply::Refused(Refusal::NotWritable)
            }
        },
        // The writer recorded the revocation as it made it.
        Wait::Keys { ticket } => match device.keys_outcome(ticket)? {
            KeyOutcome::Done => Reply::Done(None),
            KeyOutcome::NotFound => Reply::Refused(Refusal::NotFound),
            KeyOutcome::Lost => return Some(Settled::Lost),
            KeyOutcome::Full | KeyOutcome::Duplicate | KeyOutcome::NotWritable => {
                Reply::Refused(Refusal::NotWritable)
            }
        },
        Wait::Update { ticket } => match device.update_outcome(ticket)? {
            Ok(UpdateAnswer::View(view)) => Reply::Update(view),
            Ok(UpdateAnswer::Key { change, id }) => {
                record(Record::UpdateKey {
                    slot,
                    change,
                    key: id,
                });
                Reply::Done(None)
            }
            Ok(UpdateAnswer::Lost) => return Some(Settled::Lost),
            Err(refusal) => Reply::Refused(refusal.into()),
        },
    }))
}

/// The zone's text, copied out of the device's view for the answer to
/// borrow.
struct ZoneText {
    len: usize,
    bytes: [u8; wellfed_core::clock::zone::TEXT_MAX],
}

impl ZoneText {
    fn as_str(&self) -> &str {
        // The rule parser accepts ASCII only.
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

/// The time view, its zone left empty for the caller to fill from the
/// returned text.
fn time_view<D: Device>(device: &mut D, now_ms: u32) -> Option<(TimeView<'static>, ZoneText)> {
    let TimeSettingsResult::Current { view, .. } = device.time_settings(None) else {
        return None;
    };
    let state = device.time_state(now_ms)?;

    let mut text = ZoneText {
        len: 0,
        bytes: [0; wellfed_core::clock::zone::TEXT_MAX],
    };
    if let Some(zone) = view.settings.zone {
        let bytes = zone.text();
        text.bytes[..bytes.len()].copy_from_slice(bytes);
        text.len = bytes.len();
    }

    let clock = state.clock;
    let reading = clock.reading;
    let view = TimeView {
        generation: view.generation,
        writable: view.writable,
        zone: "",
        ntp: proto_ntp(view.settings.ntp),
        clock_ms: reading.map(|r| r.unix_ms),
        error_ms: reading.map(|r| u32::try_from(r.error_ms).unwrap_or(u32::MAX)),
        offset_s: reading.and_then(|r| r.offset_s),
        rate_ppb: clock.rate_ppb,
        rate_bound_ppb: clock.rate_bound_ppb,
        learned: clock.learned,
        syncs: clock.syncs,
        last: clock.last.map(|(origin, _)| match origin {
            Origin::Owner => proto::TimeSource::Owner.into(),
            Origin::Client => proto::TimeSource::Client.into(),
            Origin::Ntp => proto::TimeSource::Ntp.into(),
        }),
        last_age_s: clock.last.map(|(_, age_ms)| age_ms / 1000),
        ntp_last: state.ntp_last.map(|result| match result {
            NtpResult::Used => proto::NtpResult::Used.into(),
            NtpResult::Bounded => proto::NtpResult::Bounded.into(),
            NtpResult::Lost => proto::NtpResult::Lost.into(),
            NtpResult::Refused => proto::NtpResult::Refused.into(),
        }),
    };
    Some((view, text))
}

const fn proto_ntp(ntp: NtpServer) -> proto::NtpServer {
    match ntp {
        NtpServer::Off => proto::NtpServer::Off,
        NtpServer::Gateway { port } => proto::NtpServer::Gateway { port },
        NtpServer::Address { ip, port } => proto::NtpServer::Address { ip, port },
    }
}
