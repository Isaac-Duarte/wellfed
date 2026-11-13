//! Which role may send which op: the one matrix the device enforces and
//! every client gates its controls on. Every op has a class, matched with
//! no wildcard, so an op added to the API fails to compile until someone
//! decides who may send it, rather than falling to whatever a deny list
//! forgot. The device refuses a role the class does not name `Forbidden`
//! before the op runs or is converted, so a malformed op from a role that
//! may not send it is refused the same way as a well-formed one.
//!
//! The classes read the wire op alone, with nothing the device holds, so a
//! client asks the same question, [`allowed`], before it sends. An invite
//! passes a second check once the matrix lets it through: pairing refuses
//! an invite granting a role its asker may not hand out ([`may_invite`]).
//! The matrix is behavior, not wire: changing a row changes who may do what
//! on every feeder that runs it, and the table tests below pin every row.

use crate::api::{FeederOp, Op, Role, StatsSection, UpdateOp};

/// Whether a client holding `role` may send `op`.
pub const fn allowed(role: Role, op: &Op<'_>) -> bool {
    Access::of(op).allows(role)
}

/// Whether a client with role `by` may make an invite granting `role`: the
/// check pairing makes after the matrix lets `by` ask for an invite at all,
/// refusing the rest `NotAllowed`. An owner invites any role but an update
/// signer, whose key is enrolled only at the feeder; a member invites
/// members; the server invites no one.
pub const fn may_invite(by: Role, role: Role) -> bool {
    match by {
        Role::Owner => !matches!(role, Role::UpdateSigner),
        Role::Member => matches!(role, Role::Member),
        Role::Server | Role::UpdateSigner => false,
    }
}

/// Who may send an op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Every role that opens a session: an owner, a member and the server.
    Clients,
    /// An owner and a member, the people at the feeder: the portion
    /// weight, which they weigh, and the server's key does not.
    People,
    /// An owner alone: the network, the clients, restarts, updates and
    /// their keys, the time settings, the log, memory statistics, the
    /// stock backup (the owner's copy to keep) and the meal call (the
    /// owner's voice, or the clip they chose).
    Owner,
}

impl Access {
    /// The class of `op`.
    pub const fn of(op: &Op<'_>) -> Self {
        match op {
            Op::Alive
            | Op::Status
            | Op::TimeSync(_)
            | Op::ListClients
            | Op::GetTime
            | Op::GetAlerts
            // Whom a member may invite is pairing's to say.
            | Op::CreateInvite { .. }
            | Op::CreateBoundInvite { .. } => Self::Clients,
            Op::PutWifi { .. }
            | Op::RevokeClient { .. }
            | Op::Reboot
            | Op::PutTime { .. }
            | Op::TailLog { .. }
            | Op::ReadBank1 { .. } => Self::Owner,
            Op::Stats { section } => match section {
                StatsSection::Memory => Self::Owner,
                StatsSection::Counters
                | StatsSection::Flash
                | StatsSection::Power
                | StatsSection::Food
                | StatsSection::Feeds
                | StatsSection::Portions => Self::Clients,
            },
            Op::Update(op) => match op {
                UpdateOp::Offer { .. }
                | UpdateOp::Chunk { .. }
                | UpdateOp::State
                | UpdateOp::Cancel
                | UpdateOp::EnrollKey { .. }
                | UpdateOp::KeyStatement { .. } => Self::Owner,
            },
            Op::Feeder(op) => match op {
                FeederOp::GetSettings
                | FeederOp::PutSettings { .. }
                | FeederOp::SetLock { .. }
                | FeederOp::Play { .. }
                | FeederOp::Feed { .. }
                | FeederOp::CancelFeed { .. }
                | FeederOp::FeedTrace { .. }
                | FeederOp::GetCalibration
                | FeederOp::GetPlans
                | FeederOp::PutPlans { .. }
                | FeederOp::ResetDesiccant
                | FeederOp::GetCall
                | FeederOp::PlayCall
                | FeederOp::SkipMeal { .. }
                | FeederOp::UnskipMeal { .. } => Self::Clients,
                FeederOp::PutCalibration { .. } => Self::People,
                FeederOp::StartCall { .. }
                | FeederOp::CallChunk { .. }
                | FeederOp::DeleteCall
                | FeederOp::DeleteCallIf { .. } => Self::Owner,
            },
        }
    }

    /// Whether `role` may send an op of this class. An update signer's key
    /// never opens a session, so it may send nothing.
    pub const fn allows(self, role: Role) -> bool {
        match role {
            Role::Owner => true,
            Role::Member => match self {
                Self::Clients | Self::People => true,
                Self::Owner => false,
            },
            Role::Server => match self {
                Self::Clients => true,
                Self::People | Self::Owner => false,
            },
            Role::UpdateSigner => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Bowl, Calibration, Lenient, SettingsChanges, TimeChanges, TimeSync, Tune};

    /// One op of every kind, with who may send it: owner, member, server.
    /// `kind` matches every variant with no wildcard and numbers it, so a
    /// new op fails to compile here until it has a number, and the test
    /// fails until it has a row.
    const ROWS: &[(Op<'static>, [bool; 3])] = &[
        (Op::Alive, [true, true, true]),
        (Op::Status, [true, true, true]),
        (Op::CreateInvite { role: Role::Member }, [true, true, true]),
        (
            Op::CreateBoundInvite {
                role: Role::Member,
                key: [0; 32],
            },
            [true, true, true],
        ),
        (
            Op::PutWifi {
                base: 0,
                wifi: None,
            },
            [true, false, false],
        ),
        (
            Op::TimeSync(TimeSync {
                unix_ms: 0,
                offset_s: 0,
                next_change_s: 0,
                next_offset_s: 0,
            }),
            [true, true, true],
        ),
        (Op::ListClients, [true, true, true]),
        (Op::RevokeClient { key: [0; 32] }, [true, false, false]),
        (Op::Reboot, [true, false, false]),
        (Op::GetTime, [true, true, true]),
        (
            Op::PutTime {
                base: 0,
                changes: TimeChanges {
                    zone: None,
                    ntp: None,
                },
            },
            [true, false, false],
        ),
        (Op::TailLog { after: None }, [true, false, false]),
        (Op::ReadBank1 { offset: 0 }, [true, false, false]),
        (Op::GetAlerts, [true, true, true]),
        (stats(StatsSection::Counters), [true, true, true]),
        (stats(StatsSection::Memory), [true, false, false]),
        (stats(StatsSection::Flash), [true, true, true]),
        (stats(StatsSection::Power), [true, true, true]),
        (stats(StatsSection::Food), [true, true, true]),
        (stats(StatsSection::Feeds), [true, true, true]),
        (stats(StatsSection::Portions), [true, true, true]),
        (
            Op::Update(UpdateOp::Offer {
                manifest: &[],
                downgrade: None,
            }),
            [true, false, false],
        ),
        (
            Op::Update(UpdateOp::Chunk {
                offset: 0,
                data: &[],
            }),
            [true, false, false],
        ),
        (Op::Update(UpdateOp::State), [true, false, false]),
        (Op::Update(UpdateOp::Cancel), [true, false, false]),
        (
            Op::Update(UpdateOp::EnrollKey {
                key: [0; 32],
                label: "",
            }),
            [true, false, false],
        ),
        (
            Op::Update(UpdateOp::KeyStatement { statement: &[] }),
            [true, false, false],
        ),
        (Op::Feeder(FeederOp::GetSettings), [true, true, true]),
        (
            Op::Feeder(FeederOp::PutSettings {
                base: 0,
                changes: SettingsChanges {
                    lights: None,
                    sound: None,
                    volume: None,
                    lights_window: None,
                    sound_window: None,
                    desiccant_days: None,
                    meal_reminder: None,
                },
            }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::SetLock { locked: true }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::Play { tune: Tune::Chime }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::Feed { portions: 1 }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::CancelFeed { feed: 0 }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::FeedTrace { feed: 0, offset: 0 }),
            [true, true, true],
        ),
        (Op::Feeder(FeederOp::GetCalibration), [true, true, true]),
        (
            Op::Feeder(FeederOp::PutCalibration {
                base: 0,
                calibration: Calibration {
                    portion_mg: None,
                    bowl: Lenient::Known(Bowl::Single),
                },
            }),
            [true, true, false],
        ),
        (Op::Feeder(FeederOp::GetPlans), [true, true, true]),
        (
            Op::Feeder(FeederOp::PutPlans {
                base: 0,
                plans: [None; crate::api::PLANS_MAX],
            }),
            [true, true, true],
        ),
        (Op::Feeder(FeederOp::ResetDesiccant), [true, true, true]),
        (Op::Feeder(FeederOp::GetCall), [true, true, true]),
        (
            Op::Feeder(FeederOp::StartCall { bytes: 0, crc: 0 }),
            [true, false, false],
        ),
        (
            Op::Feeder(FeederOp::CallChunk {
                offset: 0,
                data: &[],
            }),
            [true, false, false],
        ),
        (Op::Feeder(FeederOp::DeleteCall), [true, false, false]),
        (Op::Feeder(FeederOp::PlayCall), [true, true, true]),
        (
            Op::Feeder(FeederOp::SkipMeal { slot: 0, due_s: 0 }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::UnskipMeal { slot: 0, due_s: 0 }),
            [true, true, true],
        ),
        (
            Op::Feeder(FeederOp::DeleteCallIf { bytes: 0, crc: 0 }),
            [true, false, false],
        ),
    ];

    const fn stats(section: StatsSection) -> Op<'static> {
        Op::Stats { section }
    }

    /// Each kind of op its own number, `0..KINDS`.
    const KINDS: usize = 47;

    const fn kind(op: &Op<'_>) -> usize {
        match op {
            Op::Alive => 0,
            Op::Status => 1,
            Op::CreateInvite { .. } => 2,
            Op::PutWifi { .. } => 3,
            Op::TimeSync(_) => 4,
            Op::ListClients => 5,
            Op::RevokeClient { .. } => 6,
            Op::Reboot => 7,
            Op::GetTime => 8,
            Op::PutTime { .. } => 9,
            Op::TailLog { .. } => 10,
            Op::ReadBank1 { .. } => 11,
            Op::GetAlerts => 12,
            Op::Stats { section } => match section {
                StatsSection::Counters => 13,
                StatsSection::Memory => 14,
                StatsSection::Flash => 15,
                StatsSection::Power => 16,
                StatsSection::Food => 17,
                StatsSection::Feeds => 18,
                StatsSection::Portions => 19,
            },
            Op::Update(op) => match op {
                UpdateOp::Offer { .. } => 20,
                UpdateOp::Chunk { .. } => 21,
                UpdateOp::State => 22,
                UpdateOp::Cancel => 23,
                UpdateOp::EnrollKey { .. } => 24,
                UpdateOp::KeyStatement { .. } => 25,
            },
            Op::Feeder(op) => match op {
                FeederOp::GetSettings => 26,
                FeederOp::PutSettings { .. } => 27,
                FeederOp::SetLock { .. } => 28,
                FeederOp::Play { .. } => 29,
                FeederOp::Feed { .. } => 30,
                FeederOp::CancelFeed { .. } => 31,
                FeederOp::FeedTrace { .. } => 32,
                FeederOp::GetCalibration => 33,
                FeederOp::PutCalibration { .. } => 34,
                FeederOp::GetPlans => 35,
                FeederOp::PutPlans { .. } => 36,
                FeederOp::ResetDesiccant => 37,
                FeederOp::GetCall => 38,
                FeederOp::StartCall { .. } => 39,
                FeederOp::CallChunk { .. } => 40,
                FeederOp::DeleteCall => 41,
                FeederOp::PlayCall => 42,
                FeederOp::SkipMeal { .. } => 43,
                FeederOp::UnskipMeal { .. } => 44,
                FeederOp::DeleteCallIf { .. } => 46,
            },
            Op::CreateBoundInvite { .. } => 45,
        }
    }

    #[test]
    fn every_op_has_one_row() {
        let mut seen = [false; KINDS];
        for (op, _) in ROWS {
            let kind = kind(op);
            assert!(!seen[kind], "two rows for {op:?}");
            seen[kind] = true;
        }
        assert!(seen.iter().all(|&seen| seen), "an op without a row");
    }

    /// Who may invite whom: the asker's role down, the granted role across
    /// (owner, member, server, update signer).
    #[test]
    fn each_role_may_grant_exactly_its_invites() {
        let roles = [Role::Owner, Role::Member, Role::Server, Role::UpdateSigner];
        let rows = [
            (Role::Owner, [true, true, true, false]),
            (Role::Member, [false, true, false, false]),
            (Role::Server, [false, false, false, false]),
            (Role::UpdateSigner, [false, false, false, false]),
        ];
        for (by, may) in rows {
            for (role, may) in roles.into_iter().zip(may) {
                assert_eq!(may_invite(by, role), may, "{by:?} inviting {role:?}");
            }
        }
    }

    #[test]
    fn each_role_may_send_exactly_its_ops() {
        for (op, [owner, member, server]) in ROWS {
            assert_eq!(allowed(Role::Owner, op), *owner, "owner {op:?}");
            assert_eq!(allowed(Role::Member, op), *member, "member {op:?}");
            assert_eq!(allowed(Role::Server, op), *server, "server {op:?}");
            assert!(!allowed(Role::UpdateSigner, op), "signer {op:?}");
        }
    }
}
