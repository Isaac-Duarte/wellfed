//! Manual feeding through the loop on the board's platform, with a fake
//! feeding owner: a Dispense press the panel allows queues one portion
//! through the path a client's feed takes; readiness refuses a press and
//! records its lasting reasons; a feed plays no tune; and a tune a client
//! asked for (the real speaker on the fake codec) leaves a feed's path, its
//! cancel and its result alone, whether it plays, fails to open or stalls. A
//! client's `play` sounds through the loop at rest, and is refused with a
//! feed in flight or the sound settings against it. These passes take no
//! real time, so they say nothing of what a speaker step costs a feed's
//! gap between owner steps; `PLOWN1 steps max_gap_ms` measures that.

use std::sync::{Mutex, MutexGuard};

use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_core::clock::wall::{Origin, Sync};
use wellfed_core::schedule::{Plan, Weekdays};
use wellfed_core::store::slot::SlotIo;
use wellfed_core::tone::{PlayResult, Tune};
use wellfed_device_api::Device;
use wellfed_device_api::mailbox::{CallClip, SettingsFields, SettingsResult, SettingsView};
use wellfed_feeder::feeder::owner::POLL_MS;
use wellfed_feeder::feeder::{Feeder, Feeds, Platform, Record};
use wellfed_feeder::store::call::CallStore;
use wellfed_feeder::store::plans::PlansStore;
use wellfed_feeder_runtime::{call, events, plans};
use wellfed_fw_feeder::platform::{Driver, Plaf103};
use wellfed_proto::api::{
    Battery, Blocked, EventBody, FeedOutcome, FeedReason, FoodLevel, Readiness, Refusal, Source,
    Supply,
};
use wellfed_sdk::hal::interlock::INTERLOCK;
use wellfed_sdk::hal::ota::mailbox::{self, Request};
use wellfed_sdk::hal::power::{self, Monitor};
use wellfed_sdk::hal::sdk::fake;
use wellfed_sdk::hal::{audio, buttons, clock, settings, supervision};

/// The button queue, the settings, the speaker's slot and the event queue
/// are process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

/// A feeding owner that takes every feed it is handed, unless told why not.
#[derive(Default)]
struct Owner {
    submitted: Vec<(u8, Source)>,
    blocked: Option<Blocked>,
    below_floor: Option<bool>,
    /// A feed is queued or runs.
    busy: bool,
}

/// The plans' sectors on the fake flash, so a store opens writable.
struct PlanSectors;

impl SlotIo for PlanSectors {
    fn unit_bytes(&self) -> u32 {
        wellfed_sdk::hal::sdk::SECTOR_BYTES
    }
    fn read(&self, slot: u32, offset: u32, bytes: &mut [u8]) -> bool {
        wellfed_sdk::hal::sdk::storage_read(plans::PLANS_BASE + slot, offset, bytes)
    }
    fn program(&self, _: u32, _: u32, _: &[u8]) -> bool {
        false
    }
    fn erase(&self, _: u32) -> bool {
        false
    }
    fn writable(&self) -> bool {
        true
    }
}

/// 2026-10-28T08:00:00Z, the instant of the plans these tests publish.
const MEAL_MS: u64 = 1_793_145_600_000 + 8 * 3_600_000;
/// A 2,000-byte clip plays for 500 ms, and a call's lead adds a second.
const CLIP_BYTES: u32 = 2_000;
const LEAD_MS: u64 = 1_500;

/// One daily plan at [`MEAL_MS`]'s minute, not yet run, as the writer
/// publishes it with no write of its own: a claim stays in flight.
fn publish_plan(plays: u8) {
    let mut store = PlansStore::<PlanSectors>::default();
    store.open(PlanSectors);
    assert!(store.writable());
    store.plans.table[0] = Some(Plan {
        minute: 8 * 60,
        portions: 1,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: plays,
    });
    store.plans.marks[0] = u32::try_from(MEAL_MS / 1000).unwrap() - 1;
    plans::publish(&store, true);
}

/// A stored clip. The fake flash's bytes play as whatever they decode to.
fn publish_clip() {
    let mut store = CallStore::<PlanSectors>::default();
    store.clip = CallClip::Stored {
        bytes: CLIP_BYTES,
        crc: 0,
    };
    call::publish(&store, true);
}

impl Feeds for Owner {
    fn feeding(&self) -> bool {
        false
    }

    fn busy(&self) -> bool {
        self.busy
    }

    fn blocked(&self) -> Option<Blocked> {
        self.blocked
    }

    fn below_floor(&self) -> Option<bool> {
        self.below_floor
    }

    fn high_water(&self) -> Option<u32> {
        Some(0)
    }

    fn submit(&mut self, portions: u8, by: Source) -> Result<u32, Refusal> {
        self.submitted.push((portions, by));
        Ok(u32::try_from(self.submitted.len()).unwrap())
    }

    fn reserved(&self, _feed: u32) -> Option<Result<(), Refusal>> {
        Some(Ok(()))
    }

    fn cancel(&self, _feed: u32) -> bool {
        true
    }
}

struct Rig {
    _serial: MutexGuard<'static, ()>,
    feeder: Box<Feeder<Plaf103>>,
    owner: Owner,
    now: u32,
    lines: Vec<String>,
}

impl Rig {
    /// Sound on, so a tune that should not play would.
    fn new() -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fake::reset_hardware();
        buttons::reset();
        settings::reset();
        audio::reset();
        events::reset();
        clock::reset();
        plans::reset();
        call::reset();
        mailbox::reset();
        supervision::reset();
        INTERLOCK.reset();
        settings::publish(SettingsView {
            lights: true,
            sound: true,
            volume: 50,
            generation: 1,
            writable: true,
            ..Default::default()
        });
        let mut rig = Self {
            _serial: serial,
            feeder: Box::new(Feeder::new(Plaf103::new())),
            owner: Owner::default(),
            now: 0,
            lines: Vec::new(),
        };
        rig.pass();
        rig
    }

    /// One pass `POLL_MS` on, with `gestures` queued; its delay.
    fn pass_with(&mut self, gestures: &[Gesture]) -> u32 {
        for &gesture in gestures {
            buttons::inject(gesture);
        }
        self.now += POLL_MS;
        // The wall clock reads SYSTIMER, at 32,768 ticks a second.
        let ticks = u32::try_from(u64::from(self.now) * 32_768 / 1000).unwrap();
        fake::with(|h| {
            h.uptime_ms = self.now;
            h.raw_ticks = ticks;
        });
        let lines = &mut self.lines;
        self.feeder
            .pass(self.now, &mut self.owner, |r: Record<Driver>| {
                lines.push(format!("{} {r}", r.tag()));
            })
    }

    fn pass(&mut self) -> u32 {
        self.pass_with(&[])
    }

    /// An owner's sync: the wall clock reads `unix_ms` now, at UTC.
    fn sync(&mut self, unix_ms: u64) {
        let sync = Sync {
            unix_ms,
            offset_s: 0,
            next_change_s: 0,
            next_offset_s: 0,
        };
        let now = self.now;
        self.feeder
            .platform_mut()
            .time_sync(now, sync, Origin::Owner)
            .unwrap();
    }

    fn wall(&mut self) -> u64 {
        let now = self.now;
        self.feeder.platform_mut().wall_ms(now).unwrap()
    }

    fn unlock(&mut self) {
        let hold = Gesture::Hold {
            chord: Buttons::of(Button::Lock),
            level: 1,
        };
        self.pass_with(&[hold]);
        assert_eq!(self.feeder.lock().map(|l| l.locked()), Some(false));
    }

    fn press_dispense(&mut self) {
        let chord = Buttons::of(Button::Dispense);
        let up = Gesture::Up {
            chord,
            held_ms: 100,
            level: 0,
        };
        self.pass_with(&[Gesture::Down { chord }, up]);
    }

    fn logged(&self, line: &str) -> bool {
        self.lines.iter().any(|l| l == line)
    }

    fn logged_prefix(&self, prefix: &str) -> bool {
        self.lines.iter().any(|l| l.starts_with(prefix))
    }

    /// The queued events' bodies, oldest first, as the writer would take
    /// them.
    fn events(&self) -> Vec<EventBody> {
        let mut bodies = Vec::new();
        while let Some(queued) = events::pending() {
            bodies.push(wellfed_proto::decode_padded(&queued.body).unwrap());
            events::finish_pending(None);
        }
        bodies
    }
}

#[test]
fn a_dispense_press_queues_one_portion_through_the_owner() {
    let mut rig = Rig::new();
    rig.press_dispense();
    assert!(rig.owner.submitted.is_empty(), "the panel starts locked");

    rig.unlock();
    rig.press_dispense();
    assert_eq!(rig.owner.submitted, [(1, Source::Panel)]);
    assert!(rig.logged("PLFEED1 dispense feed=1"), "{:?}", rig.lines);

    // Sound is on, yet the feed plays nothing: the speaker stays closed.
    for _ in 0..200 {
        rig.pass();
        fake::play_audio(2);
    }
    assert!(
        !rig.lines.iter().any(|l| l.starts_with("PLAUD1")),
        "a feed plays no tune: {:?}",
        rig.lines
    );
    assert_eq!(audio::request(Tune::Coin), PlayResult::Started);
}

#[test]
fn a_press_obeys_readiness_and_a_lasting_reason_is_recorded() {
    let mut rig = Rig::new();
    rig.unlock();
    assert!(rig.logged("PLFEED1 readiness state=ready"));

    rig.owner.blocked = Some(Blocked::Fault);
    rig.press_dispense();
    assert!(rig.owner.submitted.is_empty());
    assert!(rig.logged("PLFEED1 readiness state=blocked reason=fault"));
    assert!(rig.logged("PLFEED1 dispense refused=fault"));

    // A passing reason is logged but never recorded.
    rig.owner.blocked = Some(Blocked::Joining);
    rig.pass();
    rig.owner.blocked = None;
    rig.pass();
    assert!(rig.logged("PLFEED1 readiness state=blocked reason=joining"));

    let changes: Vec<_> = rig
        .events()
        .into_iter()
        .filter(|body| matches!(body, EventBody::FeedingBlocked { .. }))
        .collect();
    assert_eq!(
        changes,
        [
            EventBody::FeedingBlocked {
                reason: Some(Blocked::Fault)
            },
            EventBody::FeedingBlocked { reason: None },
        ]
    );

    // The device API's view is the same decision.
    rig.owner.blocked = Some(Blocked::Resolving);
    let readiness = rig.feeder.device(rig.now, &mut rig.owner).readiness();
    assert_eq!(readiness, Readiness::Blocked(Blocked::Resolving));
}

/// A press while a feed runs starts nothing and is not queued; the first
/// for its reason is a `DispenseRefused` event, repeats only log lines, and
/// once the feeder is ready again the next reason's first is recorded too.
#[test]
fn a_press_during_a_feed_is_recorded_once_a_spell_and_not_queued() {
    let mut rig = Rig::new();
    rig.unlock();
    let _ = rig.events();

    rig.owner.blocked = Some(Blocked::Feeding);
    rig.press_dispense();
    rig.press_dispense();
    assert!(rig.owner.submitted.is_empty(), "nothing queued");
    let busy = rig
        .lines
        .iter()
        .filter(|l| *l == "PLFEED1 dispense refused=busy")
        .count();
    assert_eq!(busy, 2, "every press is logged: {:?}", rig.lines);

    rig.owner.blocked = None;
    rig.pass();
    rig.owner.blocked = Some(Blocked::Resolving);
    rig.press_dispense();
    assert!(rig.owner.submitted.is_empty());

    let refused: Vec<_> = rig
        .events()
        .into_iter()
        .filter(|body| matches!(body, EventBody::DispenseRefused { .. }))
        .collect();
    assert_eq!(
        refused,
        [
            EventBody::DispenseRefused {
                reason: Blocked::Feeding
            },
            EventBody::DispenseRefused {
                reason: Blocked::Resolving
            },
        ]
    );
}

/// A feed that ends straight into another reason, with no pass that sees
/// the feeder ready, still records the first press under the new reason:
/// only a repeat of the reason last recorded is left to the log.
#[test]
fn a_press_under_a_new_reason_is_recorded_without_a_ready_pass() {
    let mut rig = Rig::new();
    rig.unlock();
    let _ = rig.events();

    rig.owner.blocked = Some(Blocked::Feeding);
    rig.press_dispense();
    rig.owner.blocked = Some(Blocked::Battery);
    rig.press_dispense();
    rig.press_dispense();
    rig.owner.blocked = Some(Blocked::Feeding);
    rig.press_dispense();
    assert!(rig.owner.submitted.is_empty(), "nothing queued");

    let refused: Vec<_> = rig
        .events()
        .into_iter()
        .filter(|body| matches!(body, EventBody::DispenseRefused { .. }))
        .collect();
    assert_eq!(
        refused,
        [
            EventBody::DispenseRefused {
                reason: Blocked::Feeding
            },
            EventBody::DispenseRefused {
                reason: Blocked::Battery
            },
            EventBody::DispenseRefused {
                reason: Blocked::Feeding
            },
        ]
    );
}

/// Every source asks for 1 to 48 portions, through the one queue: anything
/// else is refused by name and never reaches the owner.
#[test]
fn a_feed_outside_one_to_48_portions_never_reaches_the_owner() {
    let mut rig = Rig::new();
    let by = Source::Client([7; 4]);

    for asked in [0, 49, u8::MAX] {
        let feed = rig.feeder.device(rig.now, &mut rig.owner).feed(asked, by);
        assert_eq!(feed, Err(Refusal::Portions { asked }));
    }
    assert!(rig.owner.submitted.is_empty());

    let feed = rig.feeder.device(rig.now, &mut rig.owner).feed(48, by);
    assert_eq!(feed, Ok(1));
    assert_eq!(rig.owner.submitted, [(48, by)]);
}

#[test]
fn a_client_feed_plays_no_tune() {
    let mut rig = Rig::new();
    let by = Source::Client([7; 4]);

    let feed = rig.feeder.device(rig.now, &mut rig.owner).feed(1, by);
    assert_eq!(feed, Ok(1));
    assert_eq!(rig.owner.submitted, [(1, by)]);
    for _ in 0..200 {
        rig.pass();
        fake::play_audio(2);
    }

    assert!(
        !rig.lines.iter().any(|l| l.starts_with("PLAUD1")),
        "a feed plays no tune: {:?}",
        rig.lines
    );
    assert_eq!(audio::request(Tune::Coin), PlayResult::Started);
}

/// A client's `play`, as the device API asks it.
fn play(rig: &mut Rig, tune: Tune) -> PlayResult {
    rig.feeder.device(rig.now, &mut rig.owner).play(tune)
}

fn publish_sound(sound: bool, volume: u8) {
    settings::publish(SettingsView {
        lights: true,
        sound,
        volume,
        generation: 2,
        writable: true,
        ..Default::default()
    });
}

fn audio_lines(rig: &Rig) -> Vec<&str> {
    rig.lines
        .iter()
        .filter(|l| l.starts_with("PLAUD1"))
        .map(String::as_str)
        .collect()
}

#[test]
fn a_client_s_tune_plays_through_the_loop_and_a_second_waits_its_turn() {
    let mut rig = Rig::new();

    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Started);
    assert_eq!(
        play(&mut rig, Tune::Coin),
        PlayResult::Busy,
        "one at a time"
    );
    rig.pass();
    assert_eq!(audio_lines(&rig), ["PLAUD1 play tune=test volume=50"]);
    assert_eq!(play(&mut rig, Tune::Coin), PlayResult::Busy, "it plays");

    for _ in 0..200 {
        rig.pass();
        fake::play_audio(2);
    }
    let lines = audio_lines(&rig);
    assert!(
        lines
            .last()
            .is_some_and(|l| l.starts_with("PLAUD1 done tune=test")),
        "{lines:?}"
    );
    assert_eq!(play(&mut rig, Tune::Coin), PlayResult::Started, "it ended");
}

/// The owner's sound settings bind a test sound like any other: nothing
/// opens, and the client hears why.
#[test]
fn sound_off_or_volume_zero_answers_muted_and_stays_quiet() {
    for (sound, volume) in [(false, 50), (true, 0)] {
        let mut rig = Rig::new();
        publish_sound(sound, volume);

        assert_eq!(play(&mut rig, Tune::Test), PlayResult::Muted);
        for _ in 0..10 {
            rig.pass();
        }
        assert_eq!(audio_lines(&rig), Vec::<&str>::new());
        assert_eq!(fake::with(|h| h.audio_opens), 0);
    }
}

#[test]
fn a_tune_waits_for_the_settings_to_load() {
    let mut rig = Rig::new();
    settings::reset();
    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Busy);
}

/// A feed queued or running is silent: the request changes nothing, and the
/// speaker stays closed. It plays again once the feed has ended.
#[test]
fn a_feed_in_flight_refuses_a_tune() {
    let mut rig = Rig::new();
    rig.owner.busy = true;

    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Busy);
    rig.pass();
    assert_eq!(audio_lines(&rig), Vec::<&str>::new());
    assert_eq!(fake::with(|h| h.audio_opens), 0);

    rig.owner.busy = false;
    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Started);
}

/// `Busy` from the loop's own check, for a speaker that is itself free: the
/// request queued nothing, and the slot takes a tune asked of it directly.
fn refused_by_the_loop(rig: &mut Rig) {
    assert_eq!(play(rig, Tune::Test), PlayResult::Busy);
    assert_eq!(audio::request(Tune::Test), PlayResult::Started);
}

/// A plan's claim commits the next feed to its meal before the owner has it.
#[test]
fn a_plan_s_claim_in_flight_refuses_a_tune() {
    let mut rig = Rig::new();
    publish_plan(0);
    rig.sync(MEAL_MS + 5_000);
    rig.pass();
    rig.pass();
    let claimed = format!("PLSCH1 claimed plan=0 due={}", MEAL_MS / 1000);
    assert!(rig.logged(&claimed), "{:?}", rig.lines);

    refused_by_the_loop(&mut rig);
}

#[test]
fn an_install_holding_the_flash_refuses_a_tune() {
    let mut rig = Rig::new();
    mailbox::request(Request::Cancel).unwrap();
    assert!(mailbox::installing());

    refused_by_the_loop(&mut rig);
}

#[test]
fn a_restart_on_its_way_refuses_a_tune() {
    let mut rig = Rig::new();
    supervision::begin_restart_grace(rig.now);

    refused_by_the_loop(&mut rig);
}

/// A rig with a plan at [`MEAL_MS`] calling once, its clock `ahead_of_lead`
/// before the call's lead begins, less the pass that reads the plan.
fn rig_ahead_of_a_call(ahead_of_lead: u64) -> Rig {
    let mut rig = Rig::new();
    publish_plan(1);
    publish_clip();
    rig.sync(MEAL_MS - LEAD_MS - ahead_of_lead);
    rig.pass();
    rig
}

/// Run past the meal on the real speaker: the call starts as its lead begins
/// and ends before its meal, and the meal is claimed at its instant. When a
/// tune asked for beforehand ended, if one played.
fn the_call_runs_on_time(rig: &mut Rig) -> Option<u64> {
    let mut tune_done = None;
    let mut call_start = None;
    let mut call_done = None;
    for _ in 0..250 {
        if rig.wall() >= MEAL_MS + 40 {
            break;
        }
        rig.pass();
        fake::play_audio(2);
        let wall = rig.wall();
        tune_done = tune_done.or(rig.logged_prefix("PLAUD1 done tune=test").then_some(wall));
        call_start = call_start.or(rig.logged_prefix("PLAUD1 play tune=call").then_some(wall));
        call_done = call_done.or(rig.logged_prefix("PLAUD1 done tune=call").then_some(wall));
    }

    let lead_at = MEAL_MS - LEAD_MS;
    let (call_start, call_done) = (call_start.unwrap(), call_done.unwrap());
    assert!(
        (lead_at..lead_at + 2 * u64::from(POLL_MS)).contains(&call_start),
        "{call_start} against {lead_at}"
    );
    assert!(call_done < MEAL_MS, "{call_done} against {MEAL_MS}");

    let due = MEAL_MS / 1000;
    assert!(rig.logged(&format!("PLSCH1 called plan=0 due={due} result=started")));
    assert!(!rig.logged(&format!("PLSCH1 called plan=0 due={due} result=busy")));
    assert!(rig.logged(&format!("PLSCH1 claimed plan=0 due={due}")));
    tune_done
}

/// The test tune holds the speaker for 2 s with its spare, so one asked for
/// 2.1 s ahead of the lead plays, ends in time, and costs the call nothing.
#[test]
fn a_call_starts_on_time_after_a_tune_that_fit_before_its_lead() {
    let mut rig = rig_ahead_of_a_call(2_100);
    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Started);

    let tune_done = the_call_runs_on_time(&mut rig).unwrap();
    assert!(tune_done < MEAL_MS - LEAD_MS, "{tune_done}");
}

/// One asked for 1.9 s ahead of the lead is refused, and never plays.
#[test]
fn a_call_starts_on_time_after_refusing_a_tune_that_would_not_fit() {
    let mut rig = rig_ahead_of_a_call(1_900);
    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Busy);

    assert_eq!(the_call_runs_on_time(&mut rig), None);
}

/// A feed's outputs may be claimed before the owner says it is busy.
#[test]
fn claimed_outputs_refuse_a_tune() {
    let mut rig = Rig::new();
    INTERLOCK.reset();
    INTERLOCK.claim_feed(u32::MAX).unwrap();

    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Busy);

    INTERLOCK.release_feed();
    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Started);
}

/// Setup mode owns the speaker for its own tunes. Starting it silences a tune
/// mid-play, so the codec is closed for setup's first; a client's tune then
/// waits for setup mode to end.
#[test]
fn setup_mode_takes_the_speaker_from_a_tune_and_refuses_another() {
    let mut rig = Rig::new();
    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Started);
    rig.pass();
    assert!(fake::with(|h| h.audio_open));

    rig.unlock();
    let hold = Gesture::Hold {
        chord: Buttons::of(Button::Lock).with(Button::Mic),
        level: 1,
    };
    rig.pass_with(&[hold]);
    assert!(rig.feeder.setup_on());
    assert!(!fake::with(|h| h.audio_open), "the tune was cut off");

    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Busy);
}

/// A flash write holds no tune back: the speaker writes nothing and reads
/// no sector the writer changes. A settings change queued for the writer
/// and a tune go together.
#[test]
fn a_queued_store_write_does_not_hold_a_tune() {
    let mut rig = Rig::new();
    let queued = settings::request(SettingsFields {
        volume: Some(60),
        ..Default::default()
    });
    assert!(
        matches!(queued, SettingsResult::Current { .. }),
        "{queued:?}"
    );
    assert!(settings::pending().is_some());

    assert_eq!(play(&mut rig, Tune::Test), PlayResult::Started);
}

/// A feed's path through a tune a client asked for first: queued, in flight
/// for 200 passes with the DMA taking `pages` a pass, cancelled through the
/// device API, and its `Fed` event recorded. Another tune is refused all the
/// way, and the loop never asks to wait longer than its poll.
fn feed_despite_the_speaker(rig: &mut Rig, pages: usize) {
    rig.unlock();
    rig.press_dispense();
    let feed = u32::try_from(rig.owner.submitted.len()).unwrap();
    assert_eq!(rig.owner.submitted.last(), Some(&(1, Source::Panel)));

    rig.owner.busy = true;
    for _ in 0..200 {
        assert!(rig.pass() <= POLL_MS, "a delay no longer than the poll");
        fake::play_audio(pages);
        assert_eq!(play(rig, Tune::Coin), PlayResult::Busy);
    }
    assert!(rig.feeder.device(rig.now, &mut rig.owner).cancel_feed(feed));
    rig.owner.busy = false;
    let fed = EventBody::Fed {
        feed,
        requested: 1,
        completed: 0,
        outcome: FeedOutcome::Cancelled,
        reason: FeedReason::Cancelled,
        hopper: FoodLevel::Present,
        by: Source::Panel,
    };
    rig.feeder.record_outcome(rig.now, fed);
    assert!(rig.events().contains(&fed));
}

#[test]
fn a_playing_tune_keeps_playing_and_the_feed_goes_on() {
    let mut rig = Rig::new();
    assert_eq!(play(&mut rig, Tune::Chime), PlayResult::Started);

    feed_despite_the_speaker(&mut rig, 2);
    assert!(
        rig.lines
            .iter()
            .any(|l| l.starts_with("PLAUD1 done tune=chime")),
        "it ended during the drive: {:?}",
        rig.lines
    );

    let tunes: Vec<_> = rig
        .lines
        .iter()
        .filter(|l| l.starts_with("PLAUD1 play"))
        .collect();
    assert_eq!(
        tunes,
        ["PLAUD1 play tune=chime volume=50"],
        "{:?}",
        rig.lines
    );
}

#[test]
fn a_codec_that_will_not_open_holds_up_nothing() {
    let mut rig = Rig::new();
    fake::with(|h| h.audio_open_fails = true);
    assert_eq!(play(&mut rig, Tune::Coin), PlayResult::Started);

    feed_despite_the_speaker(&mut rig, 2);
    assert!(
        rig.lines
            .iter()
            .any(|l| l.starts_with("PLAUD1 error stage=open")),
        "{:?}",
        rig.lines
    );
}

#[test]
fn a_stalled_ring_holds_up_nothing_and_still_ends() {
    let mut rig = Rig::new();
    assert_eq!(play(&mut rig, Tune::Coin), PlayResult::Started);

    // The DMA never takes a page: the tune cannot finish.
    feed_despite_the_speaker(&mut rig, 0);
    for _ in 0..300 {
        rig.pass();
    }
    assert!(
        rig.lines
            .iter()
            .any(|l| l.starts_with("PLAUD1 done tune=coin")),
        "the speaker gives up on its own bound: {:?}",
        rig.lines
    );
}

/// The owner's battery reason refuses a press like any passing reason:
/// logged, refused `Busy`, never a `FeedingBlocked` event.
#[test]
fn a_press_on_a_low_pack_is_refused_and_not_recorded_as_blocked() {
    let mut rig = Rig::new();
    rig.unlock();
    rig.owner.blocked = Some(Blocked::Battery);
    rig.press_dispense();
    assert!(rig.owner.submitted.is_empty());
    assert!(rig.logged("PLFEED1 readiness state=blocked reason=battery"));
    assert!(
        rig.logged("PLFEED1 dispense refused=busy"),
        "{:?}",
        rig.lines
    );
    assert!(
        !rig.events()
            .iter()
            .any(|body| matches!(body, EventBody::FeedingBlocked { .. }))
    );
}

/// The board's own samples end to end: the supply as the power monitor
/// publishes it, judged, its first reading a baseline and the adapter's
/// loss a `Supply` event with the owner's verdict.
#[test]
fn the_adapter_leaving_is_a_supply_event_with_the_pack_reading() {
    let mut rig = Rig::new();
    power::reset();
    let mut monitor = Monitor::default();
    // A pack: 2117 mV at the pin with the default calibration, 4.37 V.
    fake::with(|h| {
        h.input[0] |= 1 << 19;
        h.battery_adc = 2800;
    });
    monitor.step();
    monitor.step();
    rig.owner.below_floor = Some(false);
    rig.pass();
    let _ = rig.events();

    fake::with(|h| h.input[0] &= !(1 << 19));
    monitor.step();
    rig.pass();
    let events = rig.events();
    let supply: Vec<_> = events
        .iter()
        .filter_map(|body| match body {
            EventBody::Supply(supply) => Some(*supply),
            _ => None,
        })
        .collect();
    let [
        Supply {
            adapter: Some(false),
            battery: Battery::Reading { mv, percent },
            below_floor: Some(false),
        },
    ] = supply.as_slice()
    else {
        panic!("{events:?}");
    };
    assert!((4000..5000).contains(mv), "{mv}");
    // The pack reads on stock's curve from the same sample.
    assert!(*percent >= 60, "{percent}");
}
