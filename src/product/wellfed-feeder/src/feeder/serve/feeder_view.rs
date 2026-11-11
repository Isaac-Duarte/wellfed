//! The feeder's half of [`DeviceView`] ([`FeederDevice`]): settings, the
//! panel lock, tunes and the meal call, calibration, plans and skips, and
//! feeds with their traces. Every write here is refused `Busy` while a
//! feed holds the outputs, for the reason [`DeviceView`] gives.

use wellfed_core::clip;
use wellfed_core::tone::Tune;
use wellfed_device_api::mailbox::{
    Asked, Calibration, CalibrationResult, CallChange, CallClip, CallPlay, CallResult, CallView,
    LockFields, LockResult, MealSkip, PlansChange, PlansResult, PlayResult, SettingsFields,
    SettingsResult, Table,
};
use wellfed_device_api::{Device, FeederDevice};
use wellfed_proto::api::{self as proto, Readiness, Refusal, Source, TracePage};
use wellfed_proto::trace::PAGE_MAX;

use super::super::Feeds;
use super::super::call;
use super::super::feeding;
use super::super::platform::Platform;
use super::super::skip;
use super::DeviceView;
use super::device_view::today;

impl<P: Platform, F: Feeds> FeederDevice for DeviceView<'_, P, F> {
    fn today(&mut self, now_ms: u32) -> Option<i32> {
        today(self.platform, now_ms)
    }

    fn settings(&mut self, change: Option<Asked<SettingsFields>>) -> SettingsResult {
        let Some(Asked { change, by }) = change else {
            return self
                .platform
                .settings_request(SettingsFields::default(), None);
        };
        if change.changes() && self.outputs_claimed() {
            return SettingsResult::Busy;
        }
        let asked = self.stamp(by);
        self.platform.settings_request(change, Some(asked))
    }

    fn lock(&mut self, locked: bool) -> LockResult {
        self.lock.request(LockFields {
            locked: Some(locked),
        })
    }

    /// A client's tune, queued only when nothing else has a claim on the
    /// moment. A feed is silent by the owner's choice, so with one already
    /// in flight a client gets `Busy`. A tune already sounding is not cut
    /// when a feed comes after it (a Dispense press, a client's feed, a meal
    /// with no call or whose call has ended): it plays on through the drive,
    /// and its steps, the codec's close and its `done` record among them,
    /// run inside passes between the owner's steps and count against the
    /// gap the owner allows them. The meal call shares this path. Setup mode
    /// owns the speaker for its own tunes, and an install or a restart has
    /// the feeder's attention. A flash write is no reason to
    /// wait: a tune writes nothing and reads no sector a writer changes, and
    /// the meal call plays through writes the same way, at worst with
    /// underruns the speaker's `done` record counts.
    ///
    /// `Muted` stays the speaker's answer: a test sound means what the
    /// owner's sound settings let through, so the `sound` switch, a volume
    /// of 0 and the sound window all silence it.
    fn play(&mut self, tune: Tune) -> PlayResult {
        let feed = self.operating() || self.outputs_claimed();
        let elsewhere = self.setup || self.platform.installing() || self.platform.restarting();
        let wall_ms = self.platform.wall_ms(self.now_ms);
        if feed || elsewhere || call::crowds(self.call_lead, wall_ms, tune.duration_ms()) {
            return PlayResult::Busy;
        }
        self.platform.play(tune)
    }

    fn calibration(&mut self, change: Option<Calibration>) -> CalibrationResult {
        match change {
            None => self.platform.calibration(),
            Some(_) if self.outputs_claimed() => CalibrationResult::Busy,
            Some(calibration) => self.platform.calibration_change(calibration),
        }
    }

    /// A client's table is saved at the wall time now, so a plan it adds
    /// or moves never runs an occurrence already past.
    ///
    /// It waits while a meal's claim is being written, given back or
    /// settled. The claim, its feed and its release all act on the slot
    /// they took: a table saved between them would have the release enable
    /// a plan the owner saved disabled, or the claimed feed give the new
    /// plan's portions. A stored claim still waiting for the journal is no
    /// such claim: nothing is queued for it yet, its settle reads the claim
    /// as the store then holds it, and with a faulted journal the wait
    /// would last the whole boot.
    fn plans(&mut self, change: Option<Asked<Table>>) -> PlansResult {
        match change {
            None => self.platform.plans(),
            Some(_) if self.outputs_claimed() || self.claim => PlansResult::Busy,
            Some(Asked { change: table, by }) => {
                let asked = self.stamp(by);
                let change = PlansChange::Replace {
                    table,
                    at_s: asked.at,
                };
                self.platform.plans_change(change, Some(asked))
            }
        }
    }

    /// A skip, or its undo, waits as any store write does while a feed
    /// holds the outputs, and while the schedule holds feeding for a claim
    /// or a change of its own: the plans' one mailbox would be the
    /// schedule's anyway, and a meal it is taking is never the one to skip.
    fn skip_meal(
        &mut self,
        slot: u8,
        due_s: u32,
        skip: Asked<MealSkip>,
        now_ms: u32,
    ) -> Result<(), Refusal> {
        let Asked { change: skip, by } = skip;
        if self.outputs_claimed() || self.meal {
            return Err(Refusal::Busy);
        }
        let view = match self.platform.plans() {
            PlansResult::Current { pending: true, .. } | PlansResult::Busy => {
                return Err(Refusal::Busy);
            }
            PlansResult::ReadOnly => return Err(Refusal::NotWritable),
            PlansResult::Unserved => return Err(Refusal::Unavailable),
            PlansResult::Current { view, .. } => view,
        };
        if !view.writable {
            return Err(Refusal::NotWritable);
        }
        let wall_ms = self.platform.wall_ms(now_ms).ok_or(Refusal::NoClock)?;
        let platform = &*self.platform;
        let change = skip::change(&view, slot, due_s, skip, wall_ms, |ms| {
            platform.offset_at(ms)
        })?;

        let asked = self.stamp(by);
        match self.platform.plans_change(change, Some(asked)) {
            PlansResult::Current { pending: true, .. } => Ok(()),
            PlansResult::Current { .. } | PlansResult::Busy => Err(Refusal::Busy),
            PlansResult::ReadOnly => Err(Refusal::NotWritable),
            PlansResult::Unserved => Err(Refusal::Unavailable),
        }
    }

    /// A change to the clip waits, as any store write does, while a feed
    /// holds the outputs.
    fn call(&mut self, change: Option<CallChange>) -> CallResult {
        match change {
            None => self.platform.call(),
            Some(_) if self.outputs_claimed() => CallResult::Busy,
            Some(change) => self.platform.call_change(change),
        }
    }

    fn call_chunk(&mut self, offset: u32, data: &[u8]) -> Result<(), Refusal> {
        if self.outputs_claimed() {
            return Err(Refusal::Busy);
        }
        self.platform.call_chunk(offset, data)
    }

    /// One play of the stored clip, under the same rules as a test tune: it
    /// is refused `Busy` during a feed, setup, an install or a restart, and
    /// when it would still be sounding as a meal's call begins. Also while
    /// an upload or a delete is queued: the writer then has the clip's
    /// sectors, or would wait out the play to take them.
    fn play_call(&mut self) -> CallPlay {
        let bytes = match self.platform.call() {
            CallResult::Current { pending: true, .. } => return CallPlay::Busy,
            CallResult::Current {
                view:
                    CallView {
                        clip: CallClip::Stored { bytes, .. },
                        ..
                    },
                ..
            } => bytes,
            CallResult::Current { .. } => return CallPlay::NoClip,
            CallResult::Busy => return CallPlay::Busy,
            CallResult::ReadOnly | CallResult::Unserved => return CallPlay::Unserved,
        };

        let feed = self.operating() || self.outputs_claimed();
        let elsewhere = self.setup || self.platform.installing() || self.platform.restarting();
        let wall_ms = self.platform.wall_ms(self.now_ms);
        if feed || elsewhere || call::crowds(self.call_lead, wall_ms, clip::ms(bytes)) {
            return CallPlay::Busy;
        }

        match self.platform.play_call(1) {
            PlayResult::Started => CallPlay::Started,
            PlayResult::Muted => CallPlay::Muted,
            PlayResult::Busy => CallPlay::Busy,
        }
    }

    fn phase(&self) -> Option<proto::Phase> {
        self.feeds.marks().map(|marks| marks.phase)
    }

    fn readiness(&mut self) -> Readiness {
        let ready = feeding::readiness(
            &*self.feeds,
            self.platform.restarting(),
            self.setup,
            self.platform.installing(),
            self.battery_hold,
        );
        feeding::held(ready, self.meal)
    }

    fn feed(&mut self, portions: u8, by: Source) -> Result<u32, Refusal> {
        let ready = self.readiness();
        feeding::queue(self.feeds, ready, portions, by)
    }

    fn feed_reserved(&mut self, feed: u32) -> Option<Result<(), Refusal>> {
        self.feeds.reserved(feed)
    }

    fn cancel_feed(&mut self, feed: u32) -> bool {
        self.feeds.cancel(feed)
    }

    fn feed_trace(&mut self, feed: u32, offset: u16) -> Result<TracePage<'_>, Refusal> {
        let (page, _) = self
            .page
            .split_first_chunk_mut::<PAGE_MAX>()
            .ok_or(Refusal::Busy)?;
        let read = self.platform.trace_page(feed, offset, page)?;
        Ok(TracePage {
            summary: read.summary,
            stored: read.stored,
            offset,
            total: read.total,
            bytes: self.page.get(..read.len).unwrap_or(&[]),
        })
    }
}
