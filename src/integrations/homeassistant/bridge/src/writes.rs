//! The writes Home Assistant may ask for, and the decisions about each that
//! need no connection: whether this client may send it at all, the op it
//! becomes on what the session last read, and what a store's answer means.
//!
//! - **Gates.** A write this client's role may not make, or one the
//!   feeder's capabilities do not name, is never sent: an op the feeder does
//!   not know ends the session.
//! - **Stores.** A change to the settings, the plans or the calibration
//!   names the generation it was made on. One goes out at a time per store,
//!   and it has landed only when the feeder answers the next generation.
//!   A stale one is reported and the store read again; it is never sent
//!   again here.

use wellfed_client::plans::{self, Edit};
use wellfed_model::{
    Answer, Calibration, CalibrationView, Capabilities, ClientRole, Generation, Op, Refusal,
    SettingsPatch,
};

use crate::api::{NotSent, Tune};

/// The feeder's limits on a portion's weight, in milligrams (1-50 g).
pub const PORTION_MG: std::ops::RangeInclusive<u32> = 1_000..=50_000;

/// The feeder's limits on the desiccant reminder's period, in days.
const DESICCANT_DAYS: std::ops::RangeInclusive<u8> = 1..=90;

/// One write Home Assistant asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    Feed { portions: u8 },
    CancelFeed { feed: u32 },
    SetLock { locked: bool },
    ResetDesiccant,
    Play { tune: Tune },
    Settings(SettingsPatch),
    PlanEnabled { slot: u8, enabled: bool },
    PortionWeight { mg: u32 },
}

/// A store whose writes name the generation they were made on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Settings,
    Plans,
    Calibration,
}

/// What an outstanding write was, for reading its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A write whose answer is the whole result.
    Once,
    /// A feed: an answer that never came leaves it to be settled by its
    /// `Fed` event.
    Feed {
        portions: u8,
    },
    Store {
        store: Store,
        base: Generation,
    },
}

/// What the session has read that a write is based on.
#[derive(Clone, Copy, Debug)]
pub struct Read<'a> {
    pub settings: Option<Generation>,
    pub plans: Option<&'a wellfed_model::Plans>,
    pub calibration: Option<&'a CalibrationView>,
}

/// Whether this client may send `write` to a feeder that serves
/// `capabilities`.
pub fn gate(write: &Write, role: ClientRole, capabilities: &Capabilities) -> Result<(), NotSent> {
    let served = match write {
        Write::Feed { .. } | Write::CancelFeed { .. } => capabilities.feed,
        Write::ResetDesiccant => capabilities.desiccant,
        Write::PlanEnabled { .. } => capabilities.plans,
        Write::PortionWeight { .. } => capabilities.calibration,
        Write::SetLock { .. } | Write::Play { .. } | Write::Settings(_) => true,
    };
    // Every write here is a member's: an update signer's key may only sign.
    let allowed = role != ClientRole::UpdateSigner;

    wellfed_formats::require(allowed, || NotSent::Forbidden)?;
    wellfed_formats::require(served, || NotSent::Unsupported)
}

/// The op `write` becomes on what was `read`, and how to read its answer.
pub fn build(write: &Write, read: &Read<'_>) -> Result<(Op, Kind), NotSent> {
    let built = match *write {
        Write::Feed { portions } => {
            let valid = (1..=wellfed_model::PORTIONS_MAX).contains(&portions);
            wellfed_formats::require(valid, || NotSent::Invalid)?;
            (Op::Feed { portions }, Kind::Feed { portions })
        }
        Write::CancelFeed { feed } => (Op::CancelFeed { feed }, Kind::Once),
        Write::SetLock { locked } => (Op::SetLock { locked }, Kind::Once),
        Write::ResetDesiccant => (Op::ResetDesiccant, Kind::Once),
        Write::Play { tune } => (Op::Play { tune: tune.into() }, Kind::Once),

        Write::Settings(changes) => {
            let valid = changes.volume.is_none_or(|volume| volume <= 100)
                && changes
                    .desiccant_days
                    .is_none_or(|days| DESICCANT_DAYS.contains(&days));
            wellfed_formats::require(valid, || NotSent::Invalid)?;
            let base = read.settings.ok_or(NotSent::NotRead)?;
            (
                Op::PutSettings { base, changes },
                store(Store::Settings, base),
            )
        }
        Write::PlanEnabled { slot, enabled } => {
            let held = read.plans.ok_or(NotSent::NotRead)?;
            let plans = plans::edited(held, Edit::Enabled { slot, enabled })
                .map_err(|_| NotSent::Invalid)?;
            let base = held.generation;
            (Op::PutPlans { base, plans }, store(Store::Plans, base))
        }
        Write::PortionWeight { mg } => {
            wellfed_formats::require(PORTION_MG.contains(&mg), || NotSent::Invalid)?;
            let view = read.calibration.ok_or(NotSent::NotRead)?;
            let calibration = Calibration {
                portion_mg: Some(mg),
                bowl: view.calibration.bowl,
            };
            let base = view.generation;
            (
                Op::PutCalibration { base, calibration },
                store(Store::Calibration, base),
            )
        }
    };
    Ok(built)
}

const fn store(store: Store, base: Generation) -> Kind {
    Kind::Store { store, base }
}

/// What a store write's answer means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// The store moved to the next generation: this change is in it.
    Done,
    /// The store moved on before this change; it was not made.
    Stale,
    Refused(Refusal),
    /// No answer, or one that names another generation: it may or may not
    /// be in the store.
    Unknown,
}

/// What `answer` means for a change made on `base`.
pub fn landed(answer: Option<&Answer>, base: Generation) -> Landed {
    match answer {
        Some(Answer::Done {
            generation: Some(generation),
        }) if generation.0 == base.0.wrapping_add(1) => Landed::Done,
        Some(Answer::Refused(Refusal::Stale { .. })) => Landed::Stale,
        Some(Answer::Refused(refusal)) => Landed::Refused(*refusal),
        _ => Landed::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::{Bowl, PLANS, Plan, Plans, Weekdays};

    use super::*;

    const ALL: Capabilities = Capabilities {
        feed: true,
        update: true,
        time: true,
        log: true,
        stats: true,
        readiness: true,
        traces: true,
        calibration: true,
        power: true,
        backup: true,
        plans: true,
        desiccant: true,
        call: true,
    };

    #[test]
    fn a_member_may_make_every_write_and_an_update_signer_none() {
        let weigh = Write::PortionWeight { mg: 9_000 };
        assert_eq!(gate(&weigh, ClientRole::Member, &ALL), Ok(()));
        let feed = Write::Feed { portions: 1 };
        assert_eq!(gate(&feed, ClientRole::Member, &ALL), Ok(()));
        assert_eq!(
            gate(&feed, ClientRole::UpdateSigner, &ALL),
            Err(NotSent::Forbidden)
        );
    }

    #[test]
    fn an_op_the_feeder_does_not_serve_is_never_sent() {
        let without = |capabilities: Capabilities, write: Write| {
            gate(&write, ClientRole::Owner, &capabilities)
        };
        let cases = [
            (
                Capabilities {
                    plans: false,
                    ..ALL
                },
                Write::PlanEnabled {
                    slot: 0,
                    enabled: true,
                },
            ),
            (
                Capabilities {
                    desiccant: false,
                    ..ALL
                },
                Write::ResetDesiccant,
            ),
            (
                Capabilities {
                    calibration: false,
                    ..ALL
                },
                Write::PortionWeight { mg: 9_000 },
            ),
            (
                Capabilities { feed: false, ..ALL },
                Write::CancelFeed { feed: 1 },
            ),
        ];
        for (capabilities, write) in cases {
            assert_eq!(
                without(capabilities, write),
                Err(NotSent::Unsupported),
                "{write:?}"
            );
        }
        assert_eq!(
            without(Capabilities::V1, Write::SetLock { locked: true }),
            Ok(())
        );
    }

    #[test]
    fn a_store_write_needs_what_it_changes_read_first() {
        let unread = Read {
            settings: None,
            plans: None,
            calibration: None,
        };
        let patch = SettingsPatch {
            volume: Some(40),
            ..SettingsPatch::default()
        };
        for write in [
            Write::Settings(patch),
            Write::PlanEnabled {
                slot: 0,
                enabled: false,
            },
            Write::PortionWeight { mg: 9_000 },
        ] {
            assert_eq!(build(&write, &unread), Err(NotSent::NotRead), "{write:?}");
        }
    }

    #[test]
    fn values_outside_the_feeder_s_limits_are_not_sent() {
        let calibration = CalibrationView {
            generation: Generation(4),
            writable: true,
            calibration: Calibration {
                portion_mg: None,
                bowl: Bowl::Single,
            },
            phase: None,
        };
        let read = Read {
            settings: Some(Generation(2)),
            plans: None,
            calibration: Some(&calibration),
        };
        let loud = SettingsPatch {
            volume: Some(101),
            ..SettingsPatch::default()
        };
        let never = SettingsPatch {
            desiccant_days: Some(0),
            ..SettingsPatch::default()
        };
        for write in [
            Write::Feed { portions: 0 },
            Write::Feed { portions: 49 },
            Write::Settings(loud),
            Write::Settings(never),
            Write::PortionWeight { mg: 999 },
            Write::PortionWeight { mg: 50_001 },
        ] {
            assert_eq!(build(&write, &read), Err(NotSent::Invalid), "{write:?}");
        }

        let (op, kind) = build(&Write::PortionWeight { mg: 9_500 }, &read).unwrap();
        assert_eq!(
            op,
            Op::PutCalibration {
                base: Generation(4),
                calibration: Calibration {
                    portion_mg: Some(9_500),
                    bowl: Bowl::Single,
                },
            }
        );
        assert_eq!(kind, store(Store::Calibration, Generation(4)));
    }

    #[test]
    fn a_plan_switch_rewrites_only_its_slot_on_the_plans_read() {
        let breakfast = Plan {
            minute: 7 * 60,
            portions: 2,
            days: Weekdays::DAILY,
            enabled: true,
            call: 0,
        };
        let mut table = [None; PLANS];
        table[3] = Some(breakfast);
        let plans = Plans {
            generation: Generation(9),
            writable: true,
            plans: table,
        };
        let read = Read {
            settings: None,
            plans: Some(&plans),
            calibration: None,
        };

        let (op, kind) = build(
            &Write::PlanEnabled {
                slot: 3,
                enabled: false,
            },
            &read,
        )
        .unwrap();
        let mut expected = table;
        expected[3] = Some(Plan {
            enabled: false,
            ..breakfast
        });
        assert_eq!(
            op,
            Op::PutPlans {
                base: Generation(9),
                plans: expected,
            }
        );
        assert_eq!(kind, store(Store::Plans, Generation(9)));
        assert_eq!(
            build(
                &Write::PlanEnabled {
                    slot: 2,
                    enabled: true
                },
                &read
            ),
            Err(NotSent::Invalid),
            "an empty slot"
        );
    }

    #[test]
    fn a_store_write_landed_only_at_the_next_generation() {
        let base = Generation(5);
        let done = |generation| Answer::Done { generation };
        assert_eq!(landed(Some(&done(Some(Generation(6)))), base), Landed::Done);
        assert_eq!(
            landed(Some(&done(Some(Generation(8)))), base),
            Landed::Unknown
        );
        assert_eq!(landed(Some(&done(None)), base), Landed::Unknown);
        assert_eq!(landed(None, base), Landed::Unknown);
        let stale = Answer::Refused(Refusal::Stale {
            current: Generation(7),
        });
        assert_eq!(landed(Some(&stale), base), Landed::Stale);
        let busy = Answer::Refused(Refusal::Busy);
        assert_eq!(landed(Some(&busy), base), Landed::Refused(Refusal::Busy));
    }
}
