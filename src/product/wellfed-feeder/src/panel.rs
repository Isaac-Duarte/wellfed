//! The panel's child lock, as stock 1.1.15 keeps it.
//!
//! Stock's panel task runs every 50 ticks and counts visits (static analysis
//! and emulation of the stock handlers, [stock behavior]):
//! - Lock held for 100 visits (about 5 s) toggles the lock, once per hold.
//! - An unlocked panel relocks after 600 idle visits (about 30 s).
//! - A locked panel ignores Dispense, Mic and the Reset hold.
//!
//! This keeps those rules over the SDK's gestures. The Lock hold is the
//! button service's first hold threshold ([`BUTTONS`]), so it works whether
//! the panel is locked or not: the lock never locks the owner out of
//! unlocking. A gesture in progress holds off the relock, which then counts
//! from the release. Presses that pairing takes for a code count too, so the
//! panel does not relock under the owner's hands. A client may lock or unlock
//! over the device API; an unlock from there relocks on the same timer, as a
//! button unlock does.
//!
//! On an unlocked panel, a press of Dispense or Mic alone waits
//! [`CHORD_MS`] before it acts, since each starts one of the owner's
//! chords: Lock and Dispense held opens recovery, Lock and Mic setup mode.
//! A button that joins within the window makes it a chord, and the press
//! asks for nothing; one released alone before then acts at once. A
//! locked panel's refusal is not held: it acts on nothing either way.
//!
//! The panel starts locked at every boot. Stock's boot state is not
//! recovered, and starting locked means an unattended reboot never leaves
//! Dispense live.
//!
//! What an allowed action does (a feed, a tune, a reset) belongs to its own
//! owner; this module only decides whether the panel may ask.
//!
//! [stock behavior]: ../../../../docs/devices/plaf103/stock/README.md#panel-buttons-and-child-lock

use wellfed_core::buttons::{Button, Buttons, Config as ButtonsConfig, Gesture};
use wellfed_core::{deadline_passed, interval_valid, named_enum};
use wellfed_device_api::mailbox::Lock;

/// The button service's thresholds for the panel: one hold, at stock's
/// 5 s (100 visits of 50 ms).
pub const BUTTONS: ButtonsConfig = ButtonsConfig {
    debounce_ms: 30,
    holds_ms: [5_000, 0, 0],
    hold_count: 1,
};

/// How long a press of Dispense or Mic alone on an unlocked panel waits for
/// another button before it acts. Someone pressing a two-button chord gets
/// the second button down within about 100 ms of the first, and each must
/// pass the 30 ms debounce; 200 ms covers both with room, and a feed or a
/// tune that starts 200 ms after the press still answers it at once.
pub const CHORD_MS: u32 = 200;

/// How the panel locks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The hold level (of [`BUTTONS`]) at which Lock toggles and Reset acts.
    pub hold_level: u8,
    /// Idle time before an unlocked panel relocks.
    pub relock_ms: u32,
}

impl Config {
    /// Stock's: 600 idle visits of 50 ms.
    pub const STOCK: Self = Self {
        hold_level: 1,
        relock_ms: 30_000,
    };

    pub const fn valid(&self) -> bool {
        self.hold_level > 0 && interval_valid(self.relock_ms)
    }
}

named_enum! {
    /// What changed the lock.
    pub enum Source {
        Button = "button",
        Idle = "idle",
        Remote = "remote",
    }
}

named_enum! {
    /// What the panel asks for.
    pub enum Action {
        /// Dispense pressed: one manual feed.
        Feed = "feed",
        /// Mic pressed: stock's audio selection.
        Mic = "mic",
        /// Reset held: stock's factory restore.
        Reset = "reset",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Changed {
        lock: Lock,
        source: Source,
    },
    /// The panel asked for `action`; `allowed` is false while locked.
    Asked {
        action: Action,
        allowed: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Panel {
    config: Config,
    lock: Lock,
    /// The last activity: a gesture event, or a remote change.
    active_ms: u32,
    /// A gesture is in progress (no `Up` yet).
    pressed: bool,
    /// A press alone waiting out [`CHORD_MS`].
    waiting: Option<Waiting>,
}

/// A press of Dispense or Mic alone, which acts at `until` unless another
/// button joins it first.
#[derive(Clone, Copy, Debug)]
struct Waiting {
    action: Action,
    button: Button,
    until: u32,
}

impl Panel {
    /// The panel at boot: locked.
    pub const fn new(config: Config, now_ms: u32) -> Self {
        Self {
            config,
            lock: Lock::Locked,
            active_ms: now_ms,
            pressed: false,
            waiting: None,
        }
    }

    pub const fn lock(&self) -> Lock {
        self.lock
    }

    /// One gesture from the button service.
    pub fn gesture(&mut self, now_ms: u32, gesture: Gesture) -> Option<Event> {
        self.noted(now_ms, gesture);
        if let Some(waiting) = self.waiting.take() {
            match gesture {
                // Released alone before the window ended: no chord comes.
                Gesture::Up { chord, .. } if alone(chord, waiting.button) => {
                    return Some(self.asked(waiting.action));
                }
                // Another button joined within the window: a chord, which
                // asks for nothing.
                Gesture::Down { .. } if !deadline_passed(now_ms, waiting.until) => {}
                // The window is over and the tick has yet to act: a button
                // joined too late to make a chord, or the lone button was
                // held on. The press stands alone and asks now, as the tick
                // would have; the rest of this gesture continues the same
                // press, so it asks for nothing more.
                _ => return Some(self.asked(waiting.action)),
            }
        }

        let held = |level: u8| level == self.config.hold_level;
        let action = match gesture {
            Gesture::Hold { chord, level } if alone(chord, Button::Lock) && held(level) => {
                let lock = Lock::from_locked(!self.lock.locked());
                return Some(self.set(lock, Source::Button));
            }
            Gesture::Down { chord } if alone(chord, Button::Dispense) => {
                return self.wait(now_ms, Action::Feed, Button::Dispense);
            }
            Gesture::Down { chord } if alone(chord, Button::Mic) => {
                return self.wait(now_ms, Action::Mic, Button::Mic);
            }
            Gesture::Hold { chord, level } if alone(chord, Button::Reset) && held(level) => {
                Action::Reset
            }
            _ => return None,
        };
        Some(self.asked(action))
    }

    /// A press of `button` alone asking for `action`: refused at once while
    /// locked, else held for the chord window.
    fn wait(&mut self, now_ms: u32, action: Action, button: Button) -> Option<Event> {
        if self.lock.locked() {
            return Some(self.asked(action));
        }
        self.waiting = Some(Waiting {
            action,
            button,
            until: now_ms.wrapping_add(CHORD_MS),
        });
        None
    }

    const fn asked(&self, action: Action) -> Event {
        Event::Asked {
            action,
            allowed: !self.lock.locked(),
        }
    }

    /// A gesture pairing took (a code being keyed in) is still someone at
    /// the panel: it holds off the relock without acting.
    pub const fn noted(&mut self, now_ms: u32, gesture: Gesture) {
        self.active_ms = now_ms;
        self.pressed = !matches!(gesture, Gesture::Up { .. });
    }

    /// A signed lock or unlock from the network; `None` if already so.
    pub fn remote(&mut self, now_ms: u32, lock: Lock) -> Option<Event> {
        self.active_ms = now_ms;
        (lock != self.lock).then(|| self.set(lock, Source::Remote))
    }

    /// Acts on a press alone once its chord window is over, and relocks an
    /// unlocked panel left idle for the configured time.
    pub fn tick(&mut self, now_ms: u32) -> Option<Event> {
        if let Some(waiting) = self.waiting
            && deadline_passed(now_ms, waiting.until)
        {
            self.waiting = None;
            return Some(self.asked(waiting.action));
        }
        let idle = !self.pressed
            && deadline_passed(now_ms, self.active_ms.wrapping_add(self.config.relock_ms));
        (self.lock == Lock::Unlocked && idle).then(|| self.set(Lock::Locked, Source::Idle))
    }

    fn set(&mut self, lock: Lock, source: Source) -> Event {
        self.lock = lock;
        Event::Changed { lock, source }
    }
}

fn alone(chord: Buttons, button: Button) -> bool {
    chord == Buttons::of(button)
}
