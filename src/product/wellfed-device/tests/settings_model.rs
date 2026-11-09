//! A brute-force model of the two settings records: every sequence of
//! boots, saves, lost slots, failed writes and power cuts up to a depth,
//! run on the stores' own code (`SettingsStore`, `repair`, `save_both`,
//! `Settings::view`) over a NOR image whose writes can stop at any step and
//! whose reads can fail at a boot. The writer's order is mirrored: a boot
//! opens both records, publishes, repairs them if they share no generation,
//! and publishes again; a save is taken only while the view is writable,
//! and published unless the power was cut; a failed save that leaves the
//! records apart earns one repair later, as the writer's retry. The check,
//! over the whole sequence: no generation a view reports (and so no event,
//! which names a save's generation) ever names two different settings.
//!
//! The suite runs depth 5 (about 4 500 states, 5 s unoptimized). The
//! ignored `deep` test runs depth 7 by default (131 000 states, about 3
//! minutes unoptimized), and `WELLFED_SETTINGS_MODEL_DEPTH` sets another:
//! in release, depth 8 takes about 35 s, 9 about 2.5 minutes and 10 (12.5
//! million states) about 10 minutes.

use std::cell::RefCell;
use std::collections::HashSet;

use wellfed_core::settings::{KindSettings, NO_GENERATION};
use wellfed_core::store::slot::{Extent, Layout, RECORD_BYTES, SlotIo, Version};
use wellfed_device::store::settings::{
    SaveBoth, Settings, SettingsRecord, SettingsStore, SlotState, repair, save_both,
};

const UNIT: usize = RECORD_BYTES;
const BINDING: u64 = 0x4d4f_4445_4c00_0001;
/// Saves number their contents from here, past the defaults' volume.
const FIRST_CONTENTS: u8 = 60;

/// The kind's settings in the model: one byte, its own record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Kind(u8);

impl KindSettings for Kind {
    type Fields = ();

    const DEFAULT: Self = Self(0);
    const NO_FIELDS: Self::Fields = ();
    const OWN_RECORD: bool = true;

    fn changes((): &()) -> bool {
        false
    }

    fn valid((): &()) -> bool {
        true
    }

    fn changed(self, (): &()) -> Self {
        self
    }
}

const KIND_LAYOUT: Layout = Layout::new(*b"MODELKD1", Version::new(1, 0), 32, Extent::Unit);

#[derive(Clone, Copy, Debug)]
enum KindRecord {}

impl SettingsRecord for KindRecord {
    type Value = Kind;

    const LAYOUT: &'static Layout = &KIND_LAYOUT;

    fn encode(value: &Kind, p: &mut [u8; RECORD_BYTES]) {
        p[28] = value.0;
    }

    fn decode(p: &[u8; RECORD_BYTES]) -> Option<Kind> {
        Some(Kind(p[28]))
    }

    fn valid(_: &Kind) -> bool {
        true
    }
}

/// The record a write goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Rec {
    Device,
    Kind,
}

/// Where a record's write stops. The slot layer erases the unit, programs
/// the body, then the commit marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Stop {
    /// The erase refused, the unit untouched.
    Unchanged,
    /// The erase cut short: the first half of the unit erased, the rest
    /// as it was.
    Torn,
    /// The erase cut short the other way: the second half erased, the
    /// first as it was.
    TornTail,
    /// The erase finished, and reported failure.
    ErasedFalse,
    /// Erased, nothing programmed.
    Erased,
    /// Erased, half the body programmed.
    PartBody,
    /// The body programmed, not the marker.
    NoMarker,
    /// Everything written, and the marker's program reported failure.
    Landed,
}

const STOPS: [Stop; 8] = [
    Stop::Unchanged,
    Stop::Torn,
    Stop::TornTail,
    Stop::ErasedFalse,
    Stop::Erased,
    Stop::PartBody,
    Stop::NoMarker,
    Stop::Landed,
];

/// A write that stops at `rec` as `stop`: the writer goes on (`cut` false)
/// or the power goes (`cut` true), and nothing after it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Fault {
    rec: Rec,
    stop: Stop,
    cut: bool,
}

/// The NOR image and the fault armed on it.
struct World {
    flash: [u8; 4 * UNIT],
    fault: Option<Fault>,
    fired: bool,
    /// The power is gone: nothing reads or writes.
    dead: bool,
    /// The next read of this record fails.
    read_fails: Option<Rec>,
}

thread_local! {
    static WORLD: RefCell<World> = const {
        RefCell::new(World {
            flash: [u8::MAX; 4 * UNIT],
            fault: None,
            fired: false,
            dead: false,
            read_fails: None,
        })
    };
}

/// One record's two units: the device's are 0 and 1, the kind's 2 and 3.
#[derive(Clone, Copy, Debug)]
struct Io {
    base: usize,
}

impl Io {
    fn rec(self) -> Rec {
        if self.base == 0 {
            Rec::Device
        } else {
            Rec::Kind
        }
    }

    fn unit(self, slot: u32) -> std::ops::Range<usize> {
        let at = (self.base + slot as usize) * UNIT;
        at..at + UNIT
    }

    /// The fault armed for this record, once.
    fn armed(self, world: &World) -> Option<Fault> {
        world
            .fault
            .filter(|fault| fault.rec == self.rec() && !world.fired)
    }
}

fn fire(world: &mut World, fault: Fault) {
    world.fired = true;
    world.dead = fault.cut;
}

impl SlotIo for Io {
    fn unit_bytes(&self) -> u32 {
        u32::try_from(UNIT).unwrap()
    }

    fn read(&self, slot: u32, offset: u32, bytes: &mut [u8]) -> bool {
        WORLD.with_borrow_mut(|world| {
            if world.dead {
                return false;
            }
            if world.read_fails == Some(self.rec()) {
                world.read_fails = None;
                return false;
            }
            let at = self.unit(slot).start + offset as usize;
            bytes.copy_from_slice(&world.flash[at..at + bytes.len()]);
            true
        })
    }

    fn program(&self, slot: u32, offset: u32, bytes: &[u8]) -> bool {
        WORLD.with_borrow_mut(|world| {
            if world.dead {
                return false;
            }
            let at = self.unit(slot).start + offset as usize;
            let marker = bytes.len() == 4;
            let mut written = bytes.len();
            let mut ok = true;
            if let Some(fault) = self.armed(world) {
                let stops = match fault.stop {
                    Stop::Erased => {
                        written = 0;
                        true
                    }
                    Stop::PartBody => {
                        written = bytes.len() / 2;
                        true
                    }
                    Stop::NoMarker if marker => {
                        written = 0;
                        true
                    }
                    Stop::Landed => marker,
                    Stop::NoMarker
                    | Stop::Unchanged
                    | Stop::Torn
                    | Stop::TornTail
                    | Stop::ErasedFalse => false,
                };
                if stops {
                    ok = false;
                    fire(world, fault);
                }
            }
            for (cell, &byte) in world.flash[at..at + written].iter_mut().zip(bytes) {
                *cell &= byte;
            }
            ok
        })
    }

    fn erase(&self, slot: u32) -> bool {
        WORLD.with_borrow_mut(|world| {
            if world.dead {
                return false;
            }
            let unit = self.unit(slot);
            match self.armed(world) {
                Some(
                    fault @ Fault {
                        stop: Stop::Unchanged,
                        ..
                    },
                ) => {
                    fire(world, fault);
                    false
                }
                Some(
                    fault @ Fault {
                        stop: Stop::Torn, ..
                    },
                ) => {
                    world.flash[unit.start..unit.start + UNIT / 2].fill(u8::MAX);
                    fire(world, fault);
                    false
                }
                Some(
                    fault @ Fault {
                        stop: Stop::TornTail,
                        ..
                    },
                ) => {
                    world.flash[unit.start + UNIT / 2..unit.end].fill(u8::MAX);
                    fire(world, fault);
                    false
                }
                Some(
                    fault @ Fault {
                        stop: Stop::ErasedFalse,
                        ..
                    },
                ) => {
                    world.flash[unit].fill(u8::MAX);
                    fire(world, fault);
                    false
                }
                _ => {
                    world.flash[unit].fill(u8::MAX);
                    true
                }
            }
        })
    }
}

type DeviceStore = SettingsStore<Io>;
type KindStoreOf = SettingsStore<Io, KindRecord>;

/// What a view names: the device's volume and the kind's byte, which each
/// save sets to a number of its own.
type Contents = (u8, u8);

/// One point of a sequence: the flash, the writer's stores in memory, and
/// every generation published so far with what it named.
#[derive(Clone)]
struct State {
    flash: [u8; 4 * UNIT],
    device: DeviceStore,
    kind: KindStoreOf,
    /// The power is gone until the next boot.
    off: bool,
    /// A failed save left the records apart: the writer repairs them once
    /// more, after its pause.
    retry: bool,
    published: Vec<(u64, Contents)>,
    next: u8,
}

/// A step of a sequence.
#[derive(Clone, Copy, Debug)]
enum Event {
    /// A boot, its repair stopping as the fault says, and the first read of
    /// a record failing.
    Boot {
        fault: Option<Fault>,
        read_fails: Option<Rec>,
    },
    /// A save of new contents, stopping as the fault says.
    Save(Option<Fault>),
    /// The writer's one repair after a failed save left the records apart,
    /// stopping as the fault says. A failed save has read both records
    /// again, so this is the boot's repair without the boot.
    Retry(Option<Fault>),
    /// A record's valid slot lost (a bit flipped in its body): the newest,
    /// or the older beside it.
    Lose(Rec, Which),
}

/// Which of a record's valid slots a loss takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Which {
    Newest,
    Older,
}

/// A sequence that broke the rule.
#[derive(Debug)]
struct Collision {
    generation: u64,
    first: Contents,
    then: Contents,
}

impl State {
    fn new() -> Result<Self, Collision> {
        let mut state = Self {
            flash: [u8::MAX; 4 * UNIT],
            device: DeviceStore::default(),
            kind: KindStoreOf::default(),
            off: true,
            retry: false,
            published: Vec::new(),
            next: FIRST_CONTENTS,
        };
        state.boot(None, None)?;
        Ok(state)
    }

    /// What the writer publishes now: a generation the records share, and
    /// what it names.
    fn publish(&mut self) -> Result<(), Collision> {
        let view = self.device.settings.view(
            self.device.read(),
            self.device.writable(),
            self.kind.stored(),
        );
        if view.generation == NO_GENERATION {
            assert!(!view.writable, "a view without a generation is read-only");
            return Ok(());
        }

        let generation = u64::from(view.generation);
        let contents = (view.volume, view.kind.0);
        match self.published.iter().find(|(g, _)| *g == generation) {
            Some(&(_, first)) if first != contents => Err(Collision {
                generation,
                first,
                then: contents,
            }),
            Some(_) => Ok(()),
            None => {
                self.published.push((generation, contents));
                Ok(())
            }
        }
    }

    fn writable(&self) -> bool {
        !self.off
            && self
                .device
                .settings
                .view(
                    self.device.read(),
                    self.device.writable(),
                    self.kind.stored(),
                )
                .writable
    }

    /// Run `work` on the flash with `fault` armed: whether the fault fired
    /// (none armed counts as fired) and whether the power went.
    fn with_flash(&mut self, fault: Option<Fault>, work: impl FnOnce(&mut Self)) -> (bool, bool) {
        self.with_reads(fault, None, work)
    }

    /// [`Self::with_flash`], the first read of `read_fails` failing.
    fn with_reads(
        &mut self,
        fault: Option<Fault>,
        read_fails: Option<Rec>,
        work: impl FnOnce(&mut Self),
    ) -> (bool, bool) {
        WORLD.with_borrow_mut(|world| {
            world.flash = self.flash;
            world.fault = fault;
            world.fired = false;
            world.dead = false;
            world.read_fails = read_fails;
        });
        work(self);
        WORLD.with_borrow(|world| {
            self.flash = world.flash;
            (fault.is_none() || world.fired, world.dead)
        })
    }

    /// The writer's boot: open, publish, repair, publish.
    fn boot(&mut self, fault: Option<Fault>, read_fails: Option<Rec>) -> Result<bool, Collision> {
        let mut published = Ok(());
        self.retry = false;
        let (fired, dead) = self.with_reads(fault, read_fails, |state| {
            state.device.open(Io { base: 0 }, BINDING);
            state.kind.open(Io { base: 2 }, BINDING);
            state.off = false;
            published = state.publish();
            if published.is_ok() {
                repair(&mut state.device, &mut state.kind);
            }
        });
        published?;
        if dead {
            self.off = true;
            return Ok(fired);
        }
        self.publish()?;
        Ok(fired)
    }

    /// A client's change: new contents for both records.
    fn save(&mut self, fault: Option<Fault>) -> Result<bool, Collision> {
        let n = self.next;
        self.next += 1;
        let settings = Settings {
            volume: n,
            ..Settings::default()
        };
        let mut saved = None;
        let (fired, dead) = self.with_flash(fault, |state| {
            saved = Some(save_both(
                &mut state.device,
                &mut state.kind,
                &settings,
                &Kind(n),
            ));
        });
        if dead {
            self.off = true;
            return Ok(fired);
        }
        let failed = !matches!(saved, Some(SaveBoth::Saved(_)));
        self.retry = failed && !self.writable();
        self.publish()?;
        Ok(fired)
    }

    /// The writer's repair after a failed save's pause.
    fn retry(&mut self, fault: Option<Fault>) -> Result<bool, Collision> {
        self.retry = false;
        let (fired, dead) = self.with_flash(fault, |state| {
            repair(&mut state.device, &mut state.kind);
        });
        if dead {
            self.off = true;
            return Ok(fired);
        }
        self.publish()?;
        Ok(fired)
    }

    /// Flip a bit in the body of the record's newest valid slot, or the
    /// older valid one beside it, if it has one; the stores in memory do
    /// not notice.
    fn lose(&mut self, rec: Rec, which: Which) -> bool {
        let base = match rec {
            Rec::Device => 0,
            Rec::Kind => 2,
        };
        let mut found = None;
        self.with_flash(None, |_| {
            found = match rec {
                Rec::Device => {
                    let mut probe = DeviceStore::default();
                    probe.open(Io { base }, BINDING);
                    (probe.generation > 0).then_some((probe.slot, probe.slots))
                }
                Rec::Kind => {
                    let mut probe = KindStoreOf::default();
                    probe.open(Io { base }, BINDING);
                    (probe.generation > 0).then_some((probe.slot, probe.slots))
                }
            };
        });
        let Some((newest, slots)) = found else {
            return false;
        };
        let slot = match which {
            Which::Newest => newest,
            Which::Older if slots[(1 - newest) as usize] == SlotState::Valid => 1 - newest,
            Which::Older => return false,
        };
        self.flash[(base + slot as usize) * UNIT + 40] ^= 1;
        true
    }

    /// `event` applied, or `None` when it does not apply here (or is the
    /// same as one without its fault).
    fn apply(&self, event: Event) -> Option<Result<Self, Collision>> {
        let mut next = self.clone();
        let applied = match event {
            Event::Boot { fault, read_fails } => next.boot(fault, read_fails),
            Event::Save(_) if !self.writable() => return None,
            Event::Save(fault) => next.save(fault),
            Event::Retry(_) if !self.retry || self.off => return None,
            Event::Retry(fault) => next.retry(fault),
            Event::Lose(rec, which) => Ok(next.lose(rec, which)),
        };
        match applied {
            Ok(true) => Some(Ok(next)),
            Ok(false) => None,
            Err(collision) => Some(Err(collision)),
        }
    }

    /// The parts of the state that decide what comes next and what the
    /// check compares.
    fn key(&self) -> Vec<u8> {
        let mut key = self.flash.to_vec();
        key.push(u8::from(self.off));
        key.push(u8::from(self.retry));
        key.push(self.next);
        for (generation, settings, status, slot) in [
            (
                self.device.generation,
                self.device.settings.volume,
                self.device.status,
                self.device.slot,
            ),
            (
                self.kind.generation,
                self.kind.settings.0,
                self.kind.status,
                self.kind.slot,
            ),
        ] {
            key.extend_from_slice(&generation.to_le_bytes());
            key.push(settings);
            key.push(u8::try_from(status.code()).unwrap());
            key.push(u8::try_from(slot).unwrap());
        }
        key.push(u8::from(self.device.writable()));
        key.push(u8::from(self.kind.writable()));
        let mut published = self.published.clone();
        published.sort_unstable();
        for (generation, (device, kind)) in published {
            key.extend_from_slice(&generation.to_le_bytes());
            key.push(device);
            key.push(kind);
        }
        key
    }
}

fn events() -> Vec<Event> {
    let mut faults = vec![None];
    for rec in [Rec::Device, Rec::Kind] {
        for stop in STOPS {
            for cut in [false, true] {
                faults.push(Some(Fault { rec, stop, cut }));
            }
        }
    }

    let mut events: Vec<Event> = faults
        .iter()
        .map(|&fault| Event::Boot {
            fault,
            read_fails: None,
        })
        .collect();
    // A failed read leaves its record unreadable for the boot, so no
    // repair runs and no fault could fire beside it.
    for rec in [Rec::Device, Rec::Kind] {
        events.push(Event::Boot {
            fault: None,
            read_fails: Some(rec),
        });
    }
    events.extend(faults.iter().map(|&f| Event::Save(f)));
    events.extend(faults.iter().map(|&f| Event::Retry(f)));
    for rec in [Rec::Device, Rec::Kind] {
        for which in [Which::Newest, Which::Older] {
            events.push(Event::Lose(rec, which));
        }
    }
    events
}

/// Explore every sequence of `depth` events from a fresh feeder; the
/// first that breaks the rule, with the events that led to it.
fn explore(depth: usize) -> Result<usize, (Vec<Event>, Collision)> {
    let events = events();
    let mut seen = HashSet::new();
    let mut trail = Vec::new();
    let start = State::new().map_err(|collision| (Vec::new(), collision))?;
    walk(&start, depth, &events, &mut seen, &mut trail)?;
    Ok(seen.len())
}

fn walk(
    state: &State,
    depth: usize,
    events: &[Event],
    seen: &mut HashSet<(Vec<u8>, usize)>,
    trail: &mut Vec<Event>,
) -> Result<(), (Vec<Event>, Collision)> {
    if depth == 0 || !seen.insert((state.key(), depth)) {
        return Ok(());
    }
    for &event in events {
        if state.off && !matches!(event, Event::Boot { .. } | Event::Lose(..)) {
            continue;
        }
        trail.push(event);
        match state.apply(event) {
            None => {}
            Some(Err(collision)) => return Err((trail.clone(), collision)),
            Some(Ok(next)) => walk(&next, depth - 1, events, seen, trail)?,
        }
        trail.pop();
    }
    Ok(())
}

fn check(depth: usize) {
    match explore(depth) {
        Ok(states) => println!("depth {depth}: {states} states, no collision"),
        Err((trail, collision)) => panic!(
            "generation {} named {:?} and then {:?}, after {trail:#?}",
            collision.generation, collision.first, collision.then
        ),
    }
}

/// Deep enough for repairing at one past the larger (two lost slots, then
/// a boot) to fail here.
#[test]
fn no_generation_names_two_settings() {
    check(5);
}

/// Deeper, for the gate: `WELLFED_SETTINGS_MODEL_DEPTH` (7 by default,
/// about 3 minutes unoptimized).
/// Depth 6 already fails for a store that counts a record which fell back
/// from a torn slot as clean.
#[test]
#[ignore = "minutes; run for the gate"]
fn no_generation_names_two_settings_deep() {
    let depth = std::env::var("WELLFED_SETTINGS_MODEL_DEPTH")
        .ok()
        .and_then(|depth| depth.parse().ok())
        .unwrap_or(7);
    check(depth);
}

/// The model itself: a lost slot, a torn write and a cut are each seen,
/// so a passing run is not a model that never fails anything.
#[test]
fn the_model_reaches_records_apart() {
    let mut state = State::new().unwrap();
    assert!(state.writable());
    state.save(None).unwrap();
    state.save(None).unwrap();
    assert_eq!(state.device.generation, 2);

    // The kind's write cut after the device's landed: apart until a boot.
    let cut = Fault {
        rec: Rec::Kind,
        stop: Stop::Unchanged,
        cut: true,
    };
    assert!(state.save(Some(cut)).unwrap());
    assert!(state.off && !state.writable());
    assert!(state.boot(None, None).unwrap());
    assert_eq!(
        (state.device.generation, state.kind.generation),
        (5, 5),
        "repaired two past the larger"
    );
    assert!(state.writable());

    // A lost newest slot reads the older one, apart, and is repaired.
    assert!(state.lose(Rec::Kind, Which::Newest));
    assert!(state.boot(None, None).unwrap());
    assert_eq!((state.device.generation, state.kind.generation), (7, 7));

    // A failed kind save leaves them apart, and the retry repairs them.
    let failed = Fault {
        rec: Rec::Kind,
        stop: Stop::ErasedFalse,
        cut: false,
    };
    assert!(state.save(Some(failed)).unwrap());
    assert!(state.retry && !state.writable());
    assert!(state.retry(None).unwrap());
    assert_eq!((state.device.generation, state.kind.generation), (10, 10));
    assert!(state.writable());

    // A boot whose first read of a record fails leaves it read-only.
    assert!(state.boot(None, Some(Rec::Device)).unwrap());
    assert!(!state.writable());
}
