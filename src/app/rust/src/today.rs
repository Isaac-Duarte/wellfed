//! What the Home card and the feeder's Today tab show: the status with the
//! plans, the time settings and the feeding history a session has read. The
//! reckoning is `wellfed_client::today`'s, on the feeder's own clock and
//! zone, so Dart holds no schedule logic, the card says what the feeder will
//! do, and Home Assistant shows the same day. This file only turns it into
//! the bindings' views.

use std::time::Instant;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use wellfed_client::history::Feed;
use wellfed_client::plans::{MealAction as Action, MealState};
use wellfed_client::today::TimeSettings;
use wellfed_model::{
    Battery, ClientRole, CoreEvent, Event, EventBody, FeedOutcome, FeederEvent, Gap, Readiness,
    Source, TimeView,
};

use crate::api::feeder::TimeInfo;
use crate::api::session::{
    Card, FaultView, FedBy, FeedResult, HistoryBody, HistoryEntry, MealAction, MealStanding,
    MealView, NextMealView,
};
use crate::requests::{TimeRead, plans_view, settings_view};

/// What a session has read of the feeder, with the time as this phone read
/// it.
pub(crate) type Facts = wellfed_client::today::Facts<TimeRead>;

impl TimeSettings for TimeRead {
    fn view(&self) -> &TimeView {
        &self.view
    }
}

/// The card, once the status is in.
pub(crate) fn card(facts: &Facts) -> Option<Card> {
    let status = facts.status.as_ref()?;
    // The card is a feeder's: the app serves no other kind yet.
    let feeder = status.feeder()?;
    let time: Option<TimeInfo> = facts.time.as_ref().map(|read| read.info(Instant::now()));
    let adapter = feeder
        .supply
        .adapter
        .or(status.power.map(|power| power.adapter));
    let battery_percent = match feeder.supply.battery {
        Battery::Reading { percent, .. } => Some(percent),
        _ => None,
    };
    let blocked = match feeder.readiness {
        Readiness::Blocked(reason) => Some(reason),
        Readiness::Ready => None,
    };
    let fault = feeder.feed_fault.map(|fault| FaultView {
        feed: fault.feed,
        outcome: fault.outcome,
        reason: fault.reason,
        result: facts.fault_feed().map(result),
    });

    let today = facts.today(TimeZone::system());
    let next_meal = today.next.map(|meal| NextMealView {
        slot: meal.slot,
        at_ms: meal.at.as_millisecond(),
        minute: meal.minute,
        portions: meal.portions,
    });
    let meals = today
        .meals
        .into_iter()
        .map(|meal| MealView {
            slot: meal.slot,
            minute: meal.minute,
            portions: meal.portions,
            call: meal.call,
            standing: match meal.state {
                MealState::Fed(feed) => MealStanding::Fed {
                    result: result(&feed),
                },
                MealState::Passed => MealStanding::Passed,
                MealState::Next { at } => MealStanding::Next {
                    at_ms: at.as_millisecond(),
                },
                MealState::Later => MealStanding::Later,
                MealState::Skipped { at } => MealStanding::Skipped {
                    at_ms: at.as_millisecond(),
                },
                MealState::SkipKept => MealStanding::SkipKept,
            },
            action: meal.action.map(|action| match action {
                Action::Skip { due } => MealAction::Skip {
                    due_ms: due.as_millisecond(),
                },
                Action::Undo { due } => MealAction::Undo {
                    due_ms: due.as_millisecond(),
                },
            }),
        })
        .collect();

    Some(Card {
        build: status.build.clone(),
        food: feeder.food,
        locked: feeder.locked,
        ready: feeder.readiness == Readiness::Ready,
        blocked,
        adapter,
        battery_percent,
        clock_set: status.clock.is_some(),
        rssi: status.rssi,
        fault,
        desiccant: feeder.desiccant,
        uptime_ms: status.uptime_ms,
        bank: status.bank,
        next_meal,
        last_fed: facts.last_fed().map(result),
        meals,
        time,
        settings: facts.settings.as_ref().map(settings_view),
        plans: facts.plans.as_ref().map(plans_view),
        wifi_generation: status.wifi_generation.0,
    })
}

/// A feed as the screens show it: what it asked, what surely dropped and
/// at most, and its words.
pub(crate) fn result(fed: &Feed) -> FeedResult {
    let (confirmed, at_most) = fed.delivered.bounds();
    FeedResult {
        feed: fed.feed,
        requested: fed.requested,
        confirmed,
        at_most,
        outcome: fed.outcome,
        reason: fed.reason,
        uncertain: fed.outcome == FeedOutcome::Uncertain,
        words: fed.delivered.describe(fed.requested, fed.outcome),
        at_ms: at_ms(fed.at),
        by: fed.by.map_or(FedBy::Unknown, source_of),
        hopper: fed.hopper,
    }
}

fn at_ms(at: Option<Timestamp>) -> Option<i64> {
    at.map(Timestamp::as_millisecond)
}

fn source_of(source: Source) -> FedBy {
    match source {
        Source::Panel => FedBy::Panel,
        Source::Client { client } => FedBy::Client {
            fingerprint: client.to_string(),
        },
        Source::Feeder => FedBy::Feeder,
        Source::Schedule { plan } => FedBy::Plan { slot: plan },
        Source::Unknown => FedBy::Unknown,
    }
}

/// One event as the History tab reads it.
pub(crate) fn event_entry(event: &Event) -> HistoryEntry {
    let body = match event.body {
        EventBody::Feeder(FeederEvent::Fed { .. }) => {
            Feed::from_event(event).map_or(HistoryBody::Other, |fed| HistoryBody::Fed {
                result: result(&fed),
            })
        }
        EventBody::Feeder(FeederEvent::Missed { plan, due, reason }) => HistoryBody::Missed {
            slot: plan,
            due_ms: due.as_millisecond(),
            reason,
        },
        EventBody::Core(CoreEvent::Boot { .. }) => HistoryBody::Restarted,
        EventBody::Feeder(FeederEvent::Lock { locked, by }) => HistoryBody::Lock {
            locked,
            by: source_of(by),
        },
        EventBody::Feeder(FeederEvent::Settings { by, .. }) => {
            HistoryBody::SettingsChanged { by: source_of(by) }
        }
        EventBody::Core(CoreEvent::Wifi { by, .. }) => {
            HistoryBody::WifiChanged { by: source_of(by) }
        }
        EventBody::Feeder(FeederEvent::Plans { by, .. }) => {
            HistoryBody::PlansChanged { by: source_of(by) }
        }
        EventBody::Core(CoreEvent::Enrolled {
            client,
            role: ClientRole::UpdateSigner,
            ..
        }) => HistoryBody::UpdateKeyAdded {
            fingerprint: client.to_string(),
        },
        EventBody::Core(CoreEvent::Enrolled { client, .. }) => HistoryBody::Joined {
            fingerprint: client.to_string(),
        },
        EventBody::Core(CoreEvent::Revoked { client, by }) => HistoryBody::Revoked {
            fingerprint: client.to_string(),
            by: source_of(by),
        },
        EventBody::Core(CoreEvent::ClockSync { by, .. }) => {
            HistoryBody::ClockSet { by: source_of(by) }
        }
        EventBody::Core(CoreEvent::Lost { count }) => HistoryBody::Lost { count },
        EventBody::Feeder(FeederEvent::FeedingBlocked { reason }) => {
            HistoryBody::FeedingBlocked { reason }
        }
        EventBody::Feeder(FeederEvent::MealSoon {
            minute, portions, ..
        }) => HistoryBody::MealSoon { minute, portions },
        EventBody::Feeder(FeederEvent::Supply { adapter, .. }) => HistoryBody::Supply { adapter },
        EventBody::Feeder(FeederEvent::DispenseRefused { reason }) => {
            HistoryBody::DispenseRefused { reason }
        }
        EventBody::Feeder(FeederEvent::Food { level }) => HistoryBody::Food { level },
        EventBody::Feeder(FeederEvent::Skipped { plan, due, by }) => HistoryBody::Skipped {
            slot: plan,
            due_ms: due.as_millisecond(),
            by: source_of(by),
        },
        EventBody::Feeder(FeederEvent::Unskipped { plan, due, by }) => HistoryBody::Unskipped {
            slot: plan,
            due_ms: due.as_millisecond(),
            by: source_of(by),
        },
        // The history shows no install stops or boot records yet: the update
        // view says how an install went, and a boot reads as `Restarted`.
        // Who invited a client is the server's to act on; the `Enrolled`
        // after it shows the join.
        EventBody::Core(
            CoreEvent::UpdateStop { .. }
            | CoreEvent::UpdateAtBoot { .. }
            | CoreEvent::Restarting
            | CoreEvent::UpdateConfirmed
            | CoreEvent::Invited { .. },
        ) => HistoryBody::Other,
    };
    HistoryEntry {
        seq: event.seq,
        at_ms: at_ms(event.at),
        body,
    }
}

/// A gap as the History tab reads it: events the ring dropped unread.
pub(crate) const fn gap_entry(gap: &Gap) -> HistoryEntry {
    HistoryEntry {
        seq: gap.to,
        at_ms: None,
        body: HistoryBody::Gap {
            from: gap.from,
            to: gap.to,
        },
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::{EnrollPath, Hex};

    use super::*;

    #[test]
    fn a_key_enrolled_with_a_press_reads_as_an_update_key_and_a_client_as_joined() {
        let entry = |role, path| {
            event_entry(&Event {
                epoch: 1,
                seq: 1,
                at: None,
                body: EventBody::Core(CoreEvent::Enrolled {
                    client: Hex([0xaa, 0xbb, 0xcc, 0xdd]),
                    role,
                    path,
                }),
            })
            .body
        };

        assert!(matches!(
            entry(ClientRole::UpdateSigner, EnrollPath::Press),
            HistoryBody::UpdateKeyAdded { fingerprint } if fingerprint == "aabbccdd"
        ));
        assert!(matches!(
            entry(ClientRole::Member, EnrollPath::Invite),
            HistoryBody::Joined { .. }
        ));
    }
}
