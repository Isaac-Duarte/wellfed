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
use wellfed_client::plans::MealState;
use wellfed_client::today::TimeSettings;
use wellfed_model::{
    Battery, ClientRole, Event, EventBody, FeedOutcome, Gap, Readiness, Source, TimeView,
};

use crate::api::feeder::TimeInfo;
use crate::api::session::{
    Card, FaultView, FedBy, FeedResult, HistoryBody, HistoryEntry, MealStanding, MealView,
    NextMealView,
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
    let time: Option<TimeInfo> = facts.time.as_ref().map(|read| read.info(Instant::now()));
    let adapter = status
        .supply
        .adapter
        .or(status.power.map(|power| power.adapter));
    let battery_percent = match status.supply.battery {
        Battery::Reading { percent, .. } => Some(percent),
        _ => None,
    };
    let blocked = match status.readiness {
        Readiness::Blocked(reason) => Some(reason),
        Readiness::Ready => None,
    };
    let fault = status.feed_fault.map(|fault| FaultView {
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
            },
        })
        .collect();

    Some(Card {
        build: status.build.clone(),
        food: status.food,
        locked: status.locked,
        ready: status.readiness == Readiness::Ready,
        blocked,
        adapter,
        battery_percent,
        clock_set: status.clock.is_some(),
        rssi: status.rssi,
        fault,
        desiccant: status.desiccant,
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
    }
}

/// One event as the History tab reads it.
pub(crate) fn event_entry(event: &Event) -> HistoryEntry {
    let body = match event.body {
        EventBody::Fed { .. } => {
            Feed::from_event(event).map_or(HistoryBody::Other, |fed| HistoryBody::Fed {
                result: result(&fed),
            })
        }
        EventBody::Missed { plan, due, reason } => HistoryBody::Missed {
            slot: plan,
            due_ms: due.as_millisecond(),
            reason,
        },
        EventBody::Boot { .. } => HistoryBody::Restarted,
        EventBody::Lock { locked, by } => HistoryBody::Lock {
            locked,
            by: source_of(by),
        },
        EventBody::Settings { by, .. } => HistoryBody::SettingsChanged { by: source_of(by) },
        EventBody::Wifi { by, .. } => HistoryBody::WifiChanged { by: source_of(by) },
        EventBody::Plans { by, .. } => HistoryBody::PlansChanged { by: source_of(by) },
        EventBody::Enrolled {
            client,
            role: ClientRole::UpdateSigner,
            ..
        } => HistoryBody::UpdateKeyAdded {
            fingerprint: client.to_string(),
        },
        EventBody::Enrolled { client, .. } => HistoryBody::Joined {
            fingerprint: client.to_string(),
        },
        EventBody::Revoked { client, by } => HistoryBody::Revoked {
            fingerprint: client.to_string(),
            by: source_of(by),
        },
        EventBody::ClockSync { by, .. } => HistoryBody::ClockSet { by: source_of(by) },
        EventBody::Lost { count } => HistoryBody::Lost { count },
        EventBody::FeedingBlocked { reason } => HistoryBody::FeedingBlocked { reason },
        EventBody::MealSoon {
            minute, portions, ..
        } => HistoryBody::MealSoon { minute, portions },
        EventBody::Supply { adapter, .. } => HistoryBody::Supply { adapter },
        EventBody::DispenseRefused { reason } => HistoryBody::DispenseRefused { reason },
        EventBody::Food { level } => HistoryBody::Food { level },
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
                body: EventBody::Enrolled {
                    client: Hex([0xaa, 0xbb, 0xcc, 0xdd]),
                    role,
                    path,
                },
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
