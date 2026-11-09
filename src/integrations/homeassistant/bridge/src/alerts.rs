//! What a session makes of its feeder's alert tables, by the phone's own
//! rules: `wellfed_client::alerts::Watch` decides what opened and what
//! cleared, under the same keys the app and the server use, so Home
//! Assistant, the phone and the server's webhook never disagree about one
//! alert.
//!
//! The tracker lives as long as the integration's session handle, across
//! reconnects. Its first table seeds it: what is open then is shown, and
//! fires no event, so a restart of Home Assistant doesn't announce every
//! alert again. An alert that opened and cleared while Home Assistant was
//! down fires nothing; the server's webhook is the route that never misses
//! one.

use jiff::Timestamp;
use wellfed_client::alerts::{Change, Context, Cursor, FeederClock, Mutes, Tracked, Watch};
use wellfed_model::{AlertsView, Hex};
use wellfed_server_client::api::FeederAlerts;

use crate::api::{Alert, Alerts, Update};

#[derive(Debug)]
pub(crate) struct Tracker {
    watch: Watch,
    /// This client's fingerprint: its own joining shows, but fires nothing.
    /// Through the owner's server there is none of its own, and the
    /// server's joining fires as any other client's.
    own: Hex<4>,
    seeded: bool,
    /// The alerts last shown, so a clear names what it ends.
    shown: Vec<Tracked>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new([0; 4])
    }
}

impl Tracker {
    pub(crate) fn new(own: [u8; 4]) -> Self {
        Self {
            watch: Watch::default(),
            own: Hex(own),
            seeded: false,
            shown: Vec::new(),
        }
    }

    /// Whether the table moved since the last one applied.
    pub(crate) fn needs_read(&self, cursor: Cursor) -> bool {
        self.watch.needs_read(cursor)
    }

    /// The feeder's table `view`, read under `cursor`.
    pub(crate) fn table(
        &mut self,
        cursor: Cursor,
        view: &AlertsView,
        clock: FeederClock,
    ) -> Option<Update> {
        let mutes = Mutes::default();
        let context = self.context(&mutes, clock);
        self.watch.apply(cursor, view, &context);
        self.settle(view.unread)
    }

    /// A status that showed the table unchanged: an alert held absent past
    /// its hold ends.
    pub(crate) fn unchanged(&mut self, clock: FeederClock) -> Option<Update> {
        let mutes = Mutes::default();
        let context = self.context(&mutes, clock);
        self.watch.lapse(&context);
        let unread = self.watch.unread;
        self.settle(unread)
    }

    /// What the owner's server last read of the feeder, and whether it
    /// reaches it.
    pub(crate) fn relayed(&mut self, alerts: &FeederAlerts) -> Option<Update> {
        let mutes = Mutes::default();
        let clock = match alerts.read {
            Some(read) if read.clock_valid => FeederClock::Known,
            _ => FeederClock::Unknown,
        };
        let context = self.context(&mutes, clock);
        if let Some(read) = alerts.read {
            let cursor = Cursor {
                epoch: read.epoch,
                boot: read.boot,
                generation: read.generation,
            };
            let view = AlertsView {
                epoch: read.epoch,
                generation: read.generation,
                more: read.more,
                settling: read.settling,
                alerts: alerts.alerts.clone(),
                unread: read.unread,
            };
            self.watch.apply_relayed(cursor, &view, &context);
        }
        self.watch.relayed_reach(alerts.unreachable, &context);
        let unread = self.watch.unread;
        self.settle(unread)
    }

    fn context<'a>(&self, mutes: &'a Mutes, clock: FeederClock) -> Context<'a> {
        Context {
            now: Timestamp::now(),
            mutes,
            own: self.own,
            clock,
        }
    }

    /// Hand over every change, as an update when anything Home Assistant
    /// shows moved.
    fn settle(&mut self, unread: u8) -> Option<Update> {
        let mut opened = Vec::new();
        let mut cleared = Vec::new();
        for change in self.watch.changes() {
            match &change {
                Change::Post(tracked) => opened.push(Alert::from(tracked)),
                Change::Remove(key) => {
                    if let Some(shown) = self.shown.iter().find(|shown| shown.key == *key) {
                        cleared.push(Alert::from(shown));
                    }
                }
            }
            self.watch.confirm(&change);
        }

        let open: Vec<Tracked> = self.watch.listed().copied().collect();
        let moved = open != self.shown;
        self.shown = open;
        // The first table seeds the tracker. A server that has not read
        // the feeder yet answers with no table, which seeds nothing: what
        // its first table shows is still what was open at the start.
        let first = !self.seeded;
        if self.watch.cursor.is_some() {
            self.seeded = true;
        }
        if first {
            // What is open at the start is shown, not announced.
            opened.clear();
            cleared.clear();
        }
        if !first && !moved && opened.is_empty() && cleared.is_empty() {
            return None;
        }
        Some(Update::alerts(Alerts {
            open: self.shown.iter().map(Alert::from).collect(),
            opened,
            cleared,
            unread,
        }))
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::{Alert as Wire, Generation, OpenAlert};
    use wellfed_server_client::api::AlertsRead;

    use super::*;
    use crate::api::AlertKind;

    fn food_low() -> OpenAlert {
        OpenAlert {
            at: None,
            alert: Wire::FoodLow,
        }
    }

    fn battery_low() -> OpenAlert {
        OpenAlert {
            at: None,
            alert: Wire::BatteryBelowFloor,
        }
    }

    fn cursor(generation: u32) -> Cursor {
        Cursor {
            epoch: 1,
            boot: 1,
            generation: Generation(generation),
        }
    }

    fn view(generation: u32, alerts: &[OpenAlert]) -> AlertsView {
        AlertsView {
            epoch: 1,
            generation: Generation(generation),
            more: false,
            settling: false,
            alerts: alerts.to_vec(),
            unread: 0,
        }
    }

    fn served(generation: Option<u32>, alerts: &[OpenAlert]) -> FeederAlerts {
        FeederAlerts {
            read: generation.map(|generation| AlertsRead {
                epoch: 1,
                boot: 1,
                generation: Generation(generation),
                more: false,
                settling: false,
                unread: 0,
                clock_valid: true,
                at: Timestamp::UNIX_EPOCH,
            }),
            alerts: alerts.to_vec(),
            unreachable: None,
        }
    }

    fn kinds(alerts: &[Alert]) -> Vec<AlertKind> {
        alerts.iter().map(|alert| alert.kind).collect()
    }

    fn alerts(update: Option<Update>) -> Alerts {
        update
            .and_then(|update| update.alerts)
            .expect("an alerts update")
    }

    #[test]
    fn the_first_table_is_shown_and_fires_nothing() {
        let mut tracker = Tracker::default();

        let first = alerts(tracker.table(cursor(1), &view(1, &[food_low()]), FeederClock::Known));

        assert_eq!(kinds(&first.open), [AlertKind::FoodLow]);
        assert!(first.opened.is_empty());
        assert!(first.cleared.is_empty());
    }

    #[test]
    fn a_new_alert_fires_opened_and_an_ended_one_cleared_under_one_key() {
        let mut tracker = Tracker::default();
        tracker.table(cursor(1), &view(1, &[]), FeederClock::Known);

        let opened = alerts(tracker.table(cursor(2), &view(2, &[food_low()]), FeederClock::Known));
        assert_eq!(kinds(&opened.opened), [AlertKind::FoodLow]);
        assert!(opened.cleared.is_empty());

        let cleared = alerts(tracker.table(cursor(3), &view(3, &[]), FeederClock::Known));
        assert!(cleared.opened.is_empty());
        assert_eq!(kinds(&cleared.cleared), [AlertKind::FoodLow]);
        assert_eq!(cleared.cleared[0].key, opened.opened[0].key);
        assert!(cleared.open.is_empty());

        assert!(
            tracker.unchanged(FeederClock::Known).is_none(),
            "nothing moved, nothing to say"
        );
    }

    #[test]
    fn a_server_with_no_table_yet_leaves_its_first_table_to_seed() {
        let mut tracker = Tracker::default();

        let empty = alerts(tracker.relayed(&served(None, &[])));
        assert!(empty.open.is_empty());

        // What was open before Home Assistant came is shown, not fired.
        let first = alerts(tracker.relayed(&served(Some(4), &[food_low()])));
        assert_eq!(kinds(&first.open), [AlertKind::FoodLow]);
        assert!(first.opened.is_empty());

        let next = alerts(tracker.relayed(&served(Some(5), &[food_low(), battery_low()])));
        assert_eq!(kinds(&next.opened), [AlertKind::BatteryBelowFloor]);

        // A copy older than the table applied changes nothing.
        assert!(tracker.relayed(&served(Some(4), &[food_low()])).is_none());
    }
}
