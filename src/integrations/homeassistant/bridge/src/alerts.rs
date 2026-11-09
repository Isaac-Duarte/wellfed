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
            if self.watch.needs_read(cursor) {
                let view = AlertsView {
                    epoch: read.epoch,
                    generation: read.generation,
                    more: read.more,
                    settling: read.settling,
                    alerts: alerts.alerts.clone(),
                    unread: read.unread,
                };
                self.watch.apply(cursor, &view, &context);
            } else {
                self.watch.lapse(&context);
            }
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
        let first = !self.seeded;
        self.seeded = true;
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
