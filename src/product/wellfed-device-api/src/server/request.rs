//! One request: who may run it, what it asks of the device, and its
//! compact answer.

use super::feeder::{self, FeederAction, FeederReply};
use super::handshake::{find, fingerprint, invite};
use super::memory::{Recall, Remembered, recall};
use super::{Phase, Server, Wait, Waiting, slot_id};
use crate::access::Access;
use crate::clients::{Label, Role};
use crate::mailbox::{
    Asked, NtpServer, Origin, TimeFields, TimeResult, TimeSettingsResult, WifiResult, Zone,
};
use crate::pairing::{self, Pairing};
use crate::wifi::Credentials;
use crate::{Device, KeyChange, REBOOT_MS, Record, UpdateRequest};
use wellfed_proto::api::{
    self as proto, ClientMessage, EventBody, FrozenRefusal, Op, Refusal, RequestId, Source,
    StatsSection, UPDATE_CHUNK_MAX, UpdateOp, UpdateRefusal, UpdateView,
};

impl Server {
    /// Answer or queue one request. `None` when the answer waits on the
    /// device (it then goes out from [`Self::poll_pending`]). A feeder
    /// request, once its id, role and retry are checked, goes to the
    /// feeder's module (`feeder`). `access` is the class of the op the
    /// action came from, which decides who may send it.
    #[expect(clippy::too_many_arguments, reason = "one request's whole context")]
    pub(super) fn request<D: Device>(
        &mut self,
        index: usize,
        id: RequestId,
        access: Access,
        action: Action,
        now_ms: u32,
        device: &mut D,
        pairing: &mut Pairing,
        record: &mut impl FnMut(Record),
    ) -> Option<Reply> {
        let Phase::Open {
            key, role, memory, ..
        } = self.slots[index].phase
        else {
            return None;
        };

        if action.writes()
            && let Some(remembered) = self.memories[memory].find(&id)
        {
            record(Record::Retried {
                slot: slot_id(index),
            });
            // A write still under way, or one whose outcome is lost, goes
            // through the pending poll, which waits on the first or ends
            // this session unanswered.
            return match recall(remembered, device, pairing, now_ms) {
                Recall::Answer(reply) => Some(reply),
                Recall::Waiting | Recall::Lost => self.wait(index, id, action, Waiting::Retry),
            };
        }
        if !access.allows(proto_role(role)) {
            let outcome = Reply::Refused(Refusal::Forbidden);
            return Some(self.remember(memory, id, action, outcome));
        }
        let by = Source::Client(fingerprint(&key));
        let outcome = match action {
            Action::Feeder(action) => match device.feeder() {
                Some(feeder) => {
                    return self.feeder_request(index, id, action, memory, by, now_ms, feeder);
                }
                // Another kind of device: the retry and the role were
                // checked as for any write, and the refusal is remembered.
                None => Reply::Refused(Refusal::Unavailable),
            },
            Action::Alive => Reply::Done(None),
            Action::Status => Reply::Status,
            Action::PutWifi { base, wifi } => match put_wifi(base, wifi, by, device) {
                Ok(change) => return self.queued(index, id, action, Wait::Wifi { change }),
                Err(refusal) => Reply::Refused(refusal),
            },
            Action::TimeSync(fields) => match device.time_sync(fields, origin(role)) {
                TimeResult::Synced { jump_ms } => {
                    let jump_ms = jump_ms.map(saturate);
                    device.record(now_ms, EventBody::ClockSync { by, jump_ms });
                    Reply::Done(None)
                }
                TimeResult::Refused(_) => Reply::Refused(Refusal::Time),
            },
            Action::CreateInvite { role: invited, key } => {
                let mut secret = [0; 32];
                if !device.random(&mut secret) {
                    return Some(Reply::Refused(Refusal::Busy));
                }
                let event = pairing.invite(now_ms, role, invited, key, secret);
                record(Record::Pairing(event));
                match event {
                    pairing::Event::Invited { id: invite_id } => {
                        let remembered = Remembered::Invite { id: invite_id };
                        self.memories[memory].remember(id, remembered);
                        return Some(Reply::Invite(invite(invite_id, secret)));
                    }
                    pairing::Event::Refused {
                        refusal: pairing::Refusal::Full,
                        ..
                    } => Reply::Refused(Refusal::Full),
                    _ => Reply::Refused(Refusal::Forbidden),
                }
            }
            Action::ListClients => return Some(Reply::Clients),
            Action::RevokeClient { key: revoked } => {
                if find(&self.clients, &revoked).is_none() {
                    Reply::Refused(Refusal::NotFound)
                } else if let Some(ticket) = device.keys(KeyChange::Revoke { key: revoked, by }) {
                    return self.queued(index, id, action, Wait::Keys { ticket });
                } else {
                    Reply::Refused(Refusal::Busy)
                }
            }
            Action::GetTime => match device.time_settings(None) {
                TimeSettingsResult::Current { .. } => Reply::Time,
                TimeSettingsResult::Busy => Reply::Refused(Refusal::Busy),
                TimeSettingsResult::ReadOnly | TimeSettingsResult::Unserved => {
                    Reply::Refused(Refusal::Unavailable)
                }
            },
            Action::PutTime { base, zone, ntp } => match put_time(base, zone, ntp, device) {
                Ok(digest) => return self.queued(index, id, action, Wait::Time { digest }),
                Err(refusal) => Reply::Refused(refusal),
            },
            Action::Invalid => Reply::Refused(Refusal::Invalid),
            // Each of these may write the update store; none goes in while
            // a feed is queued or driven. A cancel during an install is not
            // held off: no feed runs then.
            Action::OfferUpdate | Action::CancelUpdate | Action::UpdateKeyStatement
                if self.feeding || device.operating() =>
            {
                Reply::Refused(Refusal::Busy)
            }
            Action::OfferUpdate | Action::CancelUpdate | Action::UpdateKeyStatement => {
                match self.update_request(action, device) {
                    Ok(ticket) => return self.queued(index, id, action, Wait::Update { ticket }),
                    Err(refusal) => Reply::Refused(refusal.into()),
                }
            }
            Action::UpdateChunk { offset } => match self.update_chunk(offset, device) {
                Ok(view) => Reply::Update(view),
                Err(refusal) => Reply::Refused(refusal.into()),
            },
            Action::UpdateState => match device.update_view() {
                Some(view) => Reply::Update(view),
                None => Reply::Refused(Refusal::Unavailable),
            },
            Action::EnrollUpdateKey { key, label } => {
                let event = pairing.signer(now_ms, role);
                record(Record::Pairing(event));
                match event {
                    pairing::Event::Confirming { .. } => {
                        // Only this offer's press counts: a decision left
                        // over from an earlier one must not admit it.
                        //
                        // The one write not remembered while under way: its
                        // press is this session's, cancelled when it ends,
                        // so a retry on a new session asks for a press of
                        // its own. A known exception to retries hearing the
                        // first answer: if the first enrollment landed and
                        // nobody presses again, the retry is refused
                        // `Unconfirmed` though the key is enrolled. A false
                        // refusal is possible; a false `Done` never is, since
                        // `Done` comes only from an enrollment that landed.
                        self.signer = None;
                        let waiting = Waiting::Signer { key, label };
                        return self.wait(index, id, action, waiting);
                    }
                    pairing::Event::Refused {
                        refusal: pairing::Refusal::Busy,
                        ..
                    } => Reply::Refused(Refusal::Busy),
                    _ => Reply::Refused(Refusal::Forbidden),
                }
            }
            // A restart now would cut what the device is doing (a feed,
            // which would end uncertain).
            Action::Reboot if device.operating() => Reply::Refused(Refusal::Busy),
            Action::Reboot => {
                self.reboot_at = Some(now_ms.wrapping_add(REBOOT_MS));
                Reply::Done(None)
            }
            Action::TailLog { after } => {
                if let Phase::Open { log, .. } = &mut self.slots[index].phase {
                    *log = Some(after);
                }
                Reply::Done(None)
            }
            // The stack walk pauses the scheduler; none during a feed.
            Action::Stats {
                section: StatsSection::Memory,
            } if self.feeding || device.operating() => Reply::Refused(Refusal::Busy),
            Action::Stats { section } => return Some(Reply::Stats(section)),
            Action::ReadBank1 { offset } => return Some(Reply::Bank1 { offset }),
            Action::GetAlerts => return Some(Reply::Alerts),
        };
        Some(self.remember(memory, id, action, outcome))
    }

    /// The update op in `plain`, read again for its bytes, queued with the
    /// device. Out of line: the decoded message is a frame of its own.
    #[inline(never)]
    fn update_request<D: Device>(
        &self,
        action: Action,
        device: &mut D,
    ) -> Result<u32, FrozenRefusal> {
        let message: ClientMessage<'_> = wellfed_proto::decode(&self.plain[..self.plain_len])
            .map_err(|_| FrozenRefusal::Invalid)?;
        let ClientMessage::Request { op, .. } = message else {
            return Err(FrozenRefusal::Invalid);
        };
        let request = match (action, op) {
            (
                Action::OfferUpdate,
                Op::Update(UpdateOp::Offer {
                    manifest,
                    downgrade,
                }),
            ) => UpdateRequest::Offer {
                manifest,
                downgrade,
                device: self.identity_public.ok_or(FrozenRefusal::Busy)?,
            },
            (Action::UpdateKeyStatement, Op::Update(UpdateOp::KeyStatement { statement })) => {
                UpdateRequest::Statement { statement }
            }
            (Action::CancelUpdate, Op::Update(UpdateOp::Cancel)) => UpdateRequest::Cancel,
            _ => return Err(FrozenRefusal::Invalid),
        };
        device.update(request)
    }

    /// The chunk in `plain`, handed to the device.
    #[inline(never)]
    fn update_chunk<D: Device>(
        &self,
        offset: u32,
        device: &mut D,
    ) -> Result<UpdateView, FrozenRefusal> {
        let message: ClientMessage<'_> = wellfed_proto::decode(&self.plain[..self.plain_len])
            .map_err(|_| FrozenRefusal::Invalid)?;
        match message {
            ClientMessage::Request {
                op: Op::Update(UpdateOp::Chunk { offset: at, data }),
                ..
            } if at == offset => device.update_chunk(offset, data),
            _ => Err(FrozenRefusal::Invalid),
        }
    }
}

fn put_wifi<D: Device>(
    base: u32,
    credentials: Option<Credentials>,
    by: Source,
    device: &mut D,
) -> Result<u32, Refusal> {
    match device.wifi(None) {
        WifiResult::Current { pending: true, .. } | WifiResult::Busy => Err(Refusal::Busy),
        WifiResult::ReadOnly => Err(Refusal::NotWritable),
        WifiResult::Invalid => Err(Refusal::Invalid),
        WifiResult::Current { view, .. } if view.generation != base => Err(Refusal::Stale {
            current: view.generation,
        }),
        WifiResult::Current { .. } => match device.wifi(Some(Asked {
            change: credentials,
            by,
        })) {
            WifiResult::Current {
                view,
                pending: true,
            } => Ok(view.saved.next()),
            WifiResult::Current { .. } | WifiResult::Busy => Err(Refusal::Busy),
            WifiResult::ReadOnly => Err(Refusal::NotWritable),
            WifiResult::Invalid => Err(Refusal::Invalid),
        },
    }
}

/// Queue the time settings `base` names with the changes applied; the
/// digest of what was asked for.
fn put_time<D: Device>(
    base: u32,
    zone: Option<Option<Zone>>,
    ntp: Option<NtpServer>,
    device: &mut D,
) -> Result<u32, Refusal> {
    let view = match device.time_settings(None) {
        TimeSettingsResult::Current { pending: true, .. } | TimeSettingsResult::Busy => {
            return Err(Refusal::Busy);
        }
        TimeSettingsResult::ReadOnly => return Err(Refusal::NotWritable),
        TimeSettingsResult::Unserved => return Err(Refusal::Unavailable),
        TimeSettingsResult::Current { view, .. } => view,
    };
    if view.generation != base {
        return Err(Refusal::Stale {
            current: view.generation,
        });
    }

    let mut settings = view.settings;
    if let Some(zone) = zone {
        settings.zone = zone;
    }
    if let Some(ntp) = ntp {
        settings.ntp = ntp;
    }
    match device.time_settings(Some(settings)) {
        TimeSettingsResult::Current { pending: true, .. } => Ok(settings.digest()),
        TimeSettingsResult::Current { .. } | TimeSettingsResult::Busy => Err(Refusal::Busy),
        TimeSettingsResult::ReadOnly => Err(Refusal::NotWritable),
        TimeSettingsResult::Unserved => Err(Refusal::Unavailable),
    }
}

/// An owner's sync may step the clock; anyone else's only narrows it.
const fn origin(role: Role) -> Origin {
    match role {
        Role::Owner => Origin::Owner,
        Role::Member | Role::Server | Role::UpdateSigner => Origin::Client,
    }
}

pub const fn proto_role(role: Role) -> proto::Role {
    match role {
        Role::Owner => proto::Role::Owner,
        Role::Member => proto::Role::Member,
        Role::Server => proto::Role::Server,
        Role::UpdateSigner => proto::Role::UpdateSigner,
    }
}

const fn store_role(role: proto::Role) -> Role {
    match role {
        proto::Role::Owner => Role::Owner,
        proto::Role::Member => Role::Member,
        proto::Role::Server => Role::Server,
        proto::Role::UpdateSigner => Role::UpdateSigner,
    }
}

/// What a request asks, owned: the decoded message borrows the plaintext
/// buffer the answer reuses. The shared core's actions, then one variant
/// per kind's module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Alive,
    Status,
    PutWifi {
        base: u32,
        wifi: Option<Credentials>,
    },
    TimeSync(TimeFields),
    /// An invite, for the client with static key `key` alone if it names
    /// one.
    CreateInvite {
        role: Role,
        key: Option<[u8; 32]>,
    },
    ListClients,
    RevokeClient {
        key: [u8; 32],
    },
    Reboot,
    OfferUpdate,
    UpdateChunk {
        offset: u32,
    },
    UpdateState,
    CancelUpdate,
    EnrollUpdateKey {
        key: [u8; 32],
        label: Label,
    },
    UpdateKeyStatement,
    GetTime,
    PutTime {
        base: u32,
        /// `Some(None)` clears the zone.
        zone: Option<Option<Zone>>,
        ntp: Option<NtpServer>,
    },
    TailLog {
        after: Option<u32>,
    },
    Stats {
        section: StatsSection,
    },
    ReadBank1 {
        offset: u32,
    },
    GetAlerts,
    Feeder(FeederAction),
    /// A field failed the stores' checks before anything ran.
    Invalid,
}

impl From<Op<'_>> for Action {
    fn from(op: Op<'_>) -> Self {
        match op {
            Op::Status => Self::Status,
            Op::PutWifi { base, wifi: None } => Self::PutWifi { base, wifi: None },
            Op::PutWifi {
                base,
                wifi: Some(wifi),
            } => match Credentials::new(wifi.ssid, wifi.psk.as_bytes()) {
                Some(credentials) => Self::PutWifi {
                    base,
                    wifi: Some(credentials),
                },
                None => Self::Invalid,
            },
            Op::TimeSync(sync) => Self::TimeSync(TimeFields {
                unix_ms: sync.unix_ms,
                offset_s: sync.offset_s,
                next_change_s: sync.next_change_s,
                next_offset_s: sync.next_offset_s,
            }),
            Op::CreateInvite { role } => Self::CreateInvite {
                role: store_role(role),
                key: None,
            },
            Op::CreateBoundInvite { role, key } => Self::CreateInvite {
                role: store_role(role),
                key: Some(key),
            },
            Op::ListClients => Self::ListClients,
            Op::RevokeClient { key } => Self::RevokeClient { key },
            Op::Reboot => Self::Reboot,
            Op::Update(op) => update_action(op),
            Op::Alive => Self::Alive,
            Op::GetTime => Self::GetTime,
            Op::PutTime { base, changes } => {
                let zone = match changes.zone {
                    None => None,
                    Some("") => Some(None),
                    Some(text) => match Zone::parse(text.as_bytes()) {
                        Ok(zone) => Some(Some(zone)),
                        Err(_) => return Self::Invalid,
                    },
                };
                let ntp = changes.ntp.map(|ntp| match ntp {
                    proto::NtpServer::Off => NtpServer::Off,
                    proto::NtpServer::Gateway { port } => NtpServer::Gateway { port },
                    proto::NtpServer::Address { ip, port } => NtpServer::Address { ip, port },
                });
                if matches!(
                    ntp,
                    Some(NtpServer::Gateway { port: 0 } | NtpServer::Address { port: 0, .. })
                ) {
                    return Self::Invalid;
                }
                Self::PutTime { base, zone, ntp }
            }
            Op::TailLog { after } => Self::TailLog { after },
            Op::Stats { section } => Self::Stats { section },
            Op::ReadBank1 { offset } => Self::ReadBank1 { offset },
            Op::GetAlerts => Self::GetAlerts,
            Op::Feeder(op) => feeder::action(op),
        }
    }
}

/// An update op's action; see [`Action::from`].
fn update_action(op: UpdateOp<'_>) -> Action {
    match op {
        // The bytes stay in the plaintext buffer, read again when the op
        // runs, so the action stays small.
        UpdateOp::Offer { .. } => Action::OfferUpdate,
        UpdateOp::Chunk { offset, data } => {
            if data.is_empty() || data.len() > UPDATE_CHUNK_MAX {
                return Action::Invalid;
            }
            Action::UpdateChunk { offset }
        }
        UpdateOp::State => Action::UpdateState,
        UpdateOp::Cancel => Action::CancelUpdate,
        UpdateOp::EnrollKey { key, label } => match Label::new(label.as_bytes()) {
            Some(label) => Action::EnrollUpdateKey { key, label },
            None => Action::Invalid,
        },
        UpdateOp::KeyStatement { .. } => Action::UpdateKeyStatement,
    }
}

impl Action {
    /// What the record names a request whose op does not decode.
    pub(super) const UNKNOWN: &'static str = "unknown";
    /// The chunk op's name, which the answer's record leaves out.
    pub(super) const CHUNK: &'static str = "update-chunk";
    /// The meal call's chunk op, whose answers the records leave out too.
    pub(super) const CALL_CHUNK: &'static str = FeederAction::CALL_CHUNK;

    /// Whether the request changes something, so its id is remembered.
    pub(super) const fn writes(self) -> bool {
        match self {
            Self::Feeder(action) => action.writes(),
            _ => !matches!(
                self,
                Self::Alive
                    | Self::Status
                    | Self::ListClients
                    | Self::Invalid
                    | Self::UpdateState
                    | Self::UpdateChunk { .. }
                    | Self::GetTime
                    | Self::TailLog { .. }
                    | Self::Stats { .. }
                    | Self::ReadBank1 { .. }
                    | Self::GetAlerts
            ),
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Status => "status",
            Self::PutWifi { .. } => "put-wifi",
            Self::TimeSync(_) => "time-sync",
            Self::CreateInvite { .. } => "create-invite",
            Self::ListClients => "list-clients",
            Self::RevokeClient { .. } => "revoke-client",
            Self::Reboot => "reboot",
            Self::OfferUpdate => "offer-update",
            Self::UpdateChunk { .. } => Self::CHUNK,
            Self::UpdateState => "update-state",
            Self::CancelUpdate => "cancel-update",
            Self::EnrollUpdateKey { .. } => "enroll-update-key",
            Self::UpdateKeyStatement => "update-key-statement",
            Self::GetTime => "get-time",
            Self::PutTime { .. } => "put-time",
            Self::TailLog { .. } => "tail-log",
            Self::Stats { .. } => "stats",
            Self::ReadBank1 { .. } => "read-bank1",
            Self::GetAlerts => "get-alerts",
            Self::Feeder(action) => action.name(),
            Self::Invalid => "invalid",
        }
    }
}

/// A request's answer, compact: the status, the client list and the
/// statistics are read only when the answer is sealed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Done(Option<u32>),
    Refused(Refusal),
    Status,
    Invite(proto::Invite),
    Clients,
    Update(UpdateView),
    /// The time view, read when the answer is sealed.
    Time,
    Stats(StatsSection),
    /// A page of bank 1, read when the answer is sealed.
    Bank1 {
        offset: u32,
    },
    /// The table of open alerts, read when the answer is sealed.
    Alerts,
    /// A feeder request's answer.
    Feeder(FeederReply),
}

fn saturate(jump_ms: i64) -> i32 {
    i32::try_from(jump_ms).unwrap_or(if jump_ms < 0 { i32::MIN } else { i32::MAX })
}

/// Names for records.
pub(super) trait Name {
    fn name(&self) -> &'static str;
}

impl Name for Reply {
    fn name(&self) -> &'static str {
        match self {
            Self::Done(_) => "done",
            Self::Status => "status",
            Self::Clients => "clients",
            Self::Invite(_) => "invite",
            Self::Update(_) => "update",
            Self::Time => "time",
            Self::Stats(_) => "stats",
            Self::Bank1 { .. } => "bank1",
            Self::Alerts => "alerts",
            Self::Feeder(reply) => reply.name(),
            Self::Refused(refusal) => match refusal {
                Refusal::Forbidden => "forbidden",
                Refusal::Stale { .. } => "stale",
                Refusal::Busy => "busy",
                Refusal::Unavailable => "unavailable",
                Refusal::Invalid => "invalid",
                Refusal::NotWritable => "not-writable",
                Refusal::Full => "full",
                Refusal::NotFound => "not-found",
                Refusal::Muted => "muted",
                Refusal::Time => "time",
                Refusal::Fault => "fault",
                Refusal::Unverified => "unverified",
                Refusal::Portions { .. } => "portions",
                Refusal::NoClock => "no-clock",
                Refusal::Update(refusal) => update_refusal(*refusal),
            },
        }
    }
}

fn update_refusal(refusal: UpdateRefusal) -> &'static str {
    match refusal {
        UpdateRefusal::Manifest => "update-manifest",
        UpdateRefusal::Key => "update-key",
        UpdateRefusal::Revoked => "update-revoked",
        UpdateRefusal::Signature => "update-signature",
        UpdateRefusal::Target => "update-target",
        UpdateRefusal::Same => "update-same",
        UpdateRefusal::Downgrade => "update-downgrade",
        UpdateRefusal::Statement => "update-statement",
        UpdateRefusal::Pending => "update-pending",
        UpdateRefusal::Failed => "update-failed",
        UpdateRefusal::Fallback { .. } => "update-fallback",
        UpdateRefusal::Attempt => "update-attempt",
        UpdateRefusal::Installing => "update-installing",
        UpdateRefusal::NotInstalling => "update-not-installing",
        UpdateRefusal::Store => "update-store",
        UpdateRefusal::Unconfirmed => "update-unconfirmed",
        UpdateRefusal::Battery => "update-battery",
        UpdateRefusal::HeldAsSessionKey => "update-held-as-session-key",
    }
}
