//! The speaker. Stock 1.1.15 plays through the codec's line output and
//! holds PB23 HIGH while it plays; this port does the same, one built-in tune
//! at a time (`wellfed_core::tone`).
//!
//! A signed `play` request queues a tune in a one-slot mailbox; the audio task
//! opens the codec, enables the amplifier, streams the tune into the DMA ring,
//! waits for the ring to drain and closes everything again. The slot stays
//! taken until then, so a second request is `busy`, never queued behind. The
//! `sound` setting, a volume of 0 and the sound window (on the local clock,
//! open while the clock is invalid) mute requests, and `volume` sets
//! loudness.
//!
//! The meal call ([`request_call`]) plays the owner's clip from its
//! storage sectors (`wellfed_core::clip`) the same way, a byte read at a
//! time through the mapped flash: a read takes no lease. The settings
//! writer changes those sectors only while no call plays
//! ([`call_write_begin`]), so a call never reads a clip being rewritten.
//! A factory reset erases them without that hold: it runs on a boot that
//! starts with no clip ([`publish_call`]) and publishes none until it is
//! done.
//!
//! Nothing here touches the motor or the emitters or writes flash, and the
//! device API only reaches [`request`] and [`request_call`], through the
//! loop's own refusals (`wellfed-feeder`'s `FeederDevice::play` and
//! `FeederDevice::play_call`).

use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use wellfed_core::clip::{self, ClipPlayer};
use wellfed_core::deadline_passed;
use wellfed_core::named_enum;
pub use wellfed_core::tone::PlayResult;
use wellfed_core::tone::{Player, SAMPLE_RATE, Tune};

use crate::board::storage::CALL;
use crate::sdk::{self, IrqGuard};
use crate::{clock, settings};

/// The amplifier enable (stock writes it HIGH around playback).
pub const AMP_PIN: u32 = crate::board::pins::AMP;
const AMP_PORT: u32 = AMP_PIN >> 5;
const AMP_MASK: u32 = 1 << (AMP_PIN & 31);

// PB23 shares port B with the emitter, an indicator and the buttons, which
// the supervision timer and the LED service write from other contexts. The
// ROM's port writes and `GPIO_Init` read, modify and write the whole port,
// so each write here is made under the interrupt lock, and the mask must
// reach no pin another owner holds.
const _: () = assert!(
    AMP_PORT == 1
        && AMP_MASK & crate::stopped::PB_MASK == 0
        && AMP_MASK & crate::leds::PB_MASK == 0,
    "the amplifier enable overlaps an emitter or indicator pin"
);
pub const POLL_MS: u32 = 20;
/// Zero pages the amplifier gets to settle before the first note. 60 ms
/// swallowed the 50 ms `key` tick whole while the 160 ms `alert` beeps were
/// heard (physical, Oct 24, `fw45-code-06`), so the amplifier needs longer.
const WARM_MS: u32 = 150;
/// Longer than the ring (8 pages of 16 ms) takes to play out.
const DRAIN_MS: u32 = 400;
/// Beyond the tune's own length: a DMA that stops taking pages still ends
/// playback, so the amplifier is never left on.
const LATE_MS: u32 = 1000;

const EMPTY: u8 = 0;
/// A slot holding the meal call: this bit and its plays.
const CALL_SLOT: u8 = 0x80;

/// `EMPTY`, the queued or playing tune's code + 1, or `CALL_SLOT` with the
/// call's plays.
static SLOT: AtomicU8 = AtomicU8::new(EMPTY);
static AMP_READY: AtomicBool = AtomicBool::new(false);

/// The stored clip's stream bytes, 0 for none, as the writer publishes it.
static CALL_BYTES: AtomicU32 = AtomicU32::new(0);
/// Who holds the clip's sectors: nobody, the speaker reading them, or the
/// writer changing them.
static CALL_HELD: AtomicU8 = AtomicU8::new(HELD_FREE);
const HELD_FREE: u8 = 0;
const HELD_PLAYING: u8 = 1;
const HELD_WRITING: u8 = 2;

/// The writer's half: the clip the store now holds (`None` while it holds
/// none or an upload runs).
pub fn publish_call(bytes: Option<u32>) {
    CALL_BYTES.store(bytes.unwrap_or(0), Ordering::Release);
}

/// The writer takes the clip's sectors before changing them; false while
/// a call plays from them, and the change then waits.
pub fn call_write_begin() -> bool {
    CALL_HELD
        .compare_exchange(HELD_FREE, HELD_WRITING, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// A call plays from the clip now: a change to it would only wait.
pub fn call_playing() -> bool {
    CALL_HELD.load(Ordering::Acquire) == HELD_PLAYING
}

/// The writer gives the sectors back.
pub fn call_write_end() {
    let _ =
        CALL_HELD.compare_exchange(HELD_WRITING, HELD_FREE, Ordering::AcqRel, Ordering::Acquire);
}

/// Open the codec at `rate_hz`, then enable the amplifier; false (amplifier
/// off) when the codec would not open.
fn open_output(audio: &mut sdk::Audio, rate_hz: u32) -> bool {
    if !AMP_READY.swap(true, Ordering::AcqRel) {
        // Configure the enable as an output, LOW, before its first use.
        amp(false);
        let _guard = IrqGuard::lock();
        sdk::gpio_no_pull(AMP_PIN);
        // SAFETY: the amplifier enable, which this module owns; `GPIO_Init`
        // rewrites port B's direction, which the lock keeps from
        // interleaving with another context's write.
        unsafe { sdk::gpio_init(AMP_PIN, true) };
        // SAFETY: as above.
        unsafe { sdk::gpio_port_write(AMP_PORT, AMP_MASK, 0) };
    }
    if !audio.open(rate_hz) {
        return false;
    }
    amp(true);
    true
}

/// Amplifier off first, then the codec.
fn close_output(audio: &mut sdk::Audio) {
    amp(false);
    audio.close();
}

/// The device API's half: queue `tune` unless muted or busy.
pub fn request(tune: Tune) -> PlayResult {
    queue(tune.code() + 1)
}

/// Queue the meal call to play `plays` times (1 to `clip::PLAYS_MAX`),
/// unless muted or busy; `Busy` too while no clip is stored.
pub fn request_call(plays: u8) -> PlayResult {
    if CALL_BYTES.load(Ordering::Acquire) == 0 || !(1..=clip::PLAYS_MAX).contains(&plays) {
        return PlayResult::Busy;
    }
    queue(CALL_SLOT | plays)
}

fn queue(slot: u8) -> PlayResult {
    let Some(view) = settings::current() else {
        return PlayResult::Busy;
    };
    if !view.sound_at(local_ms()) {
        return PlayResult::Muted;
    }
    match SLOT.compare_exchange(EMPTY, slot, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => PlayResult::Started,
        Err(_) => PlayResult::Busy,
    }
}

/// The clip's stream byte at `at`, through the mapped flash.
fn call_byte(at: u32) -> Option<u8> {
    let at = clip::DATA_AT.checked_add(at)?;
    let sector = at / clip::SECTOR_BYTES;
    if sector >= CALL.count {
        return None;
    }
    let mut byte = [0];
    sdk::storage_read(CALL.first + sector, at % clip::SECTOR_BYTES, &mut byte).then_some(byte[0])
}

const _: () = assert!(CALL.count == clip::SECTORS && sdk::SECTOR_BYTES == clip::SECTOR_BYTES);

/// Local wall time now, for the sound window; `None` while the clock is
/// invalid or has no offset.
fn local_ms() -> Option<i64> {
    clock::state()
        .reading
        .and_then(|reading| reading.local_ms())
}

/// Forget everything (host tests).
pub fn reset() {
    SLOT.store(EMPTY, Ordering::Release);
    AMP_READY.store(false, Ordering::Release);
    CALL_BYTES.store(0, Ordering::Release);
    CALL_HELD.store(HELD_FREE, Ordering::Release);
}

named_enum! {
    /// What failed to start a tune (`error stage=`).
    pub enum ErrorStage {
        Open = "open",
        /// The meal call's clip was gone or being rewritten.
        Clip = "clip",
    }
}

/// What the speaker plays: a built-in tune, or the meal call `plays`
/// times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sound {
    Tune(Tune),
    Call { plays: u8 },
}

impl Sound {
    fn from_slot(slot: u8) -> Option<Self> {
        if slot & CALL_SLOT != 0 {
            return Some(Self::Call {
                plays: slot & !CALL_SLOT,
            });
        }
        slot.checked_sub(1)
            .and_then(Tune::from_code)
            .map(Self::Tune)
    }
}

/// What renders it.
#[derive(Clone, Debug)]
enum Source {
    Tone(Player),
    Clip(ClipPlayer),
}

impl Source {
    fn done(&self) -> bool {
        match self {
            Self::Tone(player) => player.done(),
            Self::Clip(player) => player.done(),
        }
    }

    fn fill(&mut self, out: &mut [i16]) {
        match self {
            Self::Tone(player) => player.fill(out),
            Self::Clip(player) => player.fill(out, call_byte),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    Play {
        tune: Tune,
        volume: u8,
    },
    Done {
        tune: Tune,
        ms: u32,
        underruns: u32,
        drained: bool,
    },
    Muted {
        tune: Tune,
    },
    Error {
        stage: ErrorStage,
    },
    /// The meal call starts: `ms` for every play together.
    CallPlay {
        plays: u8,
        ms: u32,
        volume: u8,
    },
    CallDone {
        ms: u32,
        underruns: u32,
        drained: bool,
    },
}

/// The body of a `PLAUD1` line.
pub fn describe(record: Record, f: &mut impl fmt::Write) -> fmt::Result {
    match record {
        Record::Play { tune, volume } => write!(f, "play tune={} volume={volume}", tune.name()),
        Record::Done {
            tune,
            ms,
            underruns,
            drained,
        } => write!(
            f,
            "done tune={} ms={ms} underruns={underruns} drained={}",
            tune.name(),
            u8::from(drained)
        ),
        Record::Muted { tune } => write!(f, "muted tune={}", tune.name()),
        Record::Error { stage } => write!(f, "error stage={stage}"),
        Record::CallPlay { plays, ms, volume } => {
            write!(f, "play tune=call plays={plays} ms={ms} volume={volume}")
        }
        Record::CallDone {
            ms,
            underruns,
            drained,
        } => write!(
            f,
            "done tune=call ms={ms} underruns={underruns} drained={}",
            u8::from(drained)
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub delay_ms: u32,
    pub record: Option<Record>,
}

/// The playing stages own the sound and its source, so a stage that plays
/// always has them.
#[derive(Clone, Debug, Default)]
enum Stage {
    /// The startup chime waits for the settings to load.
    #[default]
    Startup,
    Idle,
    Warm {
        sound: Sound,
        source: Source,
        until: u32,
    },
    Play {
        sound: Sound,
        source: Source,
    },
    Drain {
        sound: Sound,
        since: u32,
    },
}

#[derive(Debug, Default)]
pub struct Speaker {
    stage: Stage,
    /// The codec ring's one handle, held from the open to the close, so no
    /// page is filled against a ring another open reset.
    audio: Option<sdk::Audio>,
    started: u32,
    /// The DMA's underrun count when streaming began, then the tune's own.
    underruns: u32,
    /// How long the sound plays, from its source.
    length_ms: u32,
}

fn next(delay_ms: u32, record: Option<Record>) -> Step {
    Step { delay_ms, record }
}

fn amp(on: bool) {
    let _guard = IrqGuard::lock();
    // SAFETY: the amplifier enable, which this module owns (checked above).
    unsafe { sdk::gpio_port_write(AMP_PORT, AMP_MASK, if on { AMP_MASK } else { 0 }) };
}

impl Speaker {
    /// A speaker that skips the startup chime, for a firmware that steps it
    /// in its own loop for a while rather than in a task from boot.
    pub const fn quiet() -> Self {
        Self {
            stage: Stage::Idle,
            audio: None,
            started: 0,
            underruns: 0,
            length_ms: 0,
        }
    }

    /// Nothing queued or playing: stepping can wait.
    pub fn is_idle(&self) -> bool {
        matches!(self.stage, Stage::Idle) && SLOT.load(Ordering::Acquire) == EMPTY
    }

    /// Stop what plays and forget what waits. Setup mode takes the speaker
    /// whatever it was doing: a stage dropped mid-sound would leave the
    /// codec open and the amplifier on, and every later open would fail. The
    /// `done` record goes unsaid, since a cut sound has no length to report.
    pub fn silence(&mut self) {
        if let Stage::Warm { sound, .. } | Stage::Play { sound, .. } | Stage::Drain { sound, .. } =
            self.stage
        {
            let _ = self.finish(sound, false);
        }
        SLOT.store(EMPTY, Ordering::Release);
        self.stage = Stage::Idle;
    }

    fn finish(&mut self, sound: Sound, drained: bool) -> Record {
        match self.audio.as_mut() {
            Some(audio) => close_output(audio),
            None => amp(false),
        }
        self.audio = None;
        if matches!(sound, Sound::Call { .. }) {
            CALL_HELD.store(HELD_FREE, Ordering::Release);
        }
        SLOT.store(EMPTY, Ordering::Release);
        self.stage = Stage::Idle;
        let ms = sdk::uptime_ms().wrapping_sub(self.started);
        match sound {
            Sound::Tune(tune) => Record::Done {
                tune,
                ms,
                underruns: self.underruns,
                drained,
            },
            Sound::Call { .. } => Record::CallDone {
                ms,
                underruns: self.underruns,
                drained,
            },
        }
    }

    /// The source for `sound`, its length and the record its start makes;
    /// `None` when the call's clip cannot be read now (gone, or being
    /// rewritten).
    fn source(sound: Sound, volume: u8) -> Option<(Source, u32, Record)> {
        match sound {
            Sound::Tune(tune) => Some((
                Source::Tone(Player::new(tune, volume)),
                tune.duration_ms(),
                Record::Play { tune, volume },
            )),
            Sound::Call { plays } => {
                if CALL_HELD
                    .compare_exchange(HELD_FREE, HELD_PLAYING, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return None;
                }
                // Read only once the sectors are held: the writer
                // publishes no clip before it changes them.
                let bytes = CALL_BYTES.load(Ordering::Acquire);
                if bytes == 0 {
                    CALL_HELD.store(HELD_FREE, Ordering::Release);
                    return None;
                }
                let player = ClipPlayer::new(bytes, plays, volume);
                let ms = player.duration_ms();
                Some((
                    Source::Clip(player),
                    ms,
                    Record::CallPlay { plays, ms, volume },
                ))
            }
        }
    }

    pub fn step(&mut self) -> Step {
        let now = sdk::uptime_ms();
        match &mut self.stage {
            Stage::Startup => {
                let Some(view) = settings::current() else {
                    return next(POLL_MS * 5, None);
                };
                self.stage = Stage::Idle;
                if !view.sound_at(local_ms()) {
                    return next(POLL_MS, Some(Record::Muted { tune: Tune::Chime }));
                }
                let _ = SLOT.compare_exchange(
                    EMPTY,
                    Tune::Chime.code() + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                next(POLL_MS, None)
            }

            Stage::Idle => {
                let slot = SLOT.load(Ordering::Acquire);
                let Some(sound) = Sound::from_slot(slot) else {
                    return next(POLL_MS, None);
                };
                let volume = settings::current().map_or(0, |view| view.volume);
                let Some((source, length_ms, record)) = Self::source(sound, volume) else {
                    SLOT.store(EMPTY, Ordering::Release);
                    let stage = ErrorStage::Clip;
                    return next(POLL_MS, Some(Record::Error { stage }));
                };
                // The speaker is the ring's one user: no handle is out
                // while it idles.
                let audio = sdk::Audio::take()
                    .and_then(|mut audio| open_output(&mut audio, SAMPLE_RATE).then_some(audio));
                let Some(audio) = audio else {
                    if matches!(sound, Sound::Call { .. }) {
                        CALL_HELD.store(HELD_FREE, Ordering::Release);
                    }
                    SLOT.store(EMPTY, Ordering::Release);
                    return next(
                        POLL_MS,
                        Some(Record::Error {
                            stage: ErrorStage::Open,
                        }),
                    );
                };
                self.audio = Some(audio);
                self.started = now;
                self.length_ms = length_ms;
                self.stage = Stage::Warm {
                    sound,
                    source,
                    until: now.wrapping_add(WARM_MS),
                };
                next(POLL_MS, Some(record))
            }

            Stage::Warm {
                sound,
                source,
                until,
            } => {
                if !deadline_passed(now, *until) {
                    return next(POLL_MS, None);
                }
                self.underruns = sdk::audio_underruns();
                self.stage = Stage::Play {
                    sound: *sound,
                    source: source.clone(),
                };
                self.step()
            }

            Stage::Play { sound, source } => {
                let sound = *sound;
                let bound = WARM_MS
                    .saturating_add(self.length_ms)
                    .saturating_add(LATE_MS);
                if now.wrapping_sub(self.started) >= bound {
                    self.underruns = sdk::audio_underruns().wrapping_sub(self.underruns);
                    return next(POLL_MS, Some(self.finish(sound, false)));
                }
                if let Some(audio) = self.audio.as_mut() {
                    while let Some(mut page) = audio.page() {
                        source.fill(&mut page[..]);
                        page.submit();
                        if source.done() {
                            break;
                        }
                    }
                }
                if source.done() {
                    // Zero pages the DMA played for want of a filled one.
                    self.underruns = sdk::audio_underruns().wrapping_sub(self.underruns);
                    self.stage = Stage::Drain { sound, since: now };
                }
                next(POLL_MS, None)
            }

            Stage::Drain { sound, since } => {
                let sound = *sound;
                let drained = sdk::audio_drained();
                if drained || now.wrapping_sub(*since) >= DRAIN_MS {
                    return next(POLL_MS, Some(self.finish(sound, drained)));
                }
                next(POLL_MS, None)
            }
        }
    }
}
