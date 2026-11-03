//! The writes a screen makes with a switch or a slider, and how the session
//! carries them so a control moves on the feeder's first answer.
//!
//! A write's `Done` is the feeder saying it took the change, so the card takes
//! it at once rather than after the event and the read it brings: that read
//! still comes, and only confirms. The card takes it only on the generation
//! the write went on; a card already past that waits for the read instead.
//!
//! Settings writes go out one at a time. One asked while another is out waits,
//! merged field by field with any others waiting (the later wins), and goes on
//! the generation the one out answered with, so a quick second change is not
//! refused as stale. A waiting write goes only after a `Done`: one out that was
//! refused or never answered leaves no generation to trust, so the waiting
//! ones end unsent. Nothing that went out is ever sent again.

use tokio::sync::oneshot;
use wellfed_model::{Answer, Generation, Op, PLANS, Plan, Settings, SettingsPatch};

use crate::session::Heard;
use crate::today::Facts;

/// Where one caller hears how its request ended.
pub(crate) type Reply = oneshot::Sender<Heard>;

/// A write whose `Done` the card takes as soon as it comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Settings {
        base: Generation,
        changes: SettingsPatch,
    },
    Lock {
        locked: bool,
    },
    Plans {
        base: Generation,
        plans: [Option<Plan>; PLANS],
    },
}

impl Write {
    /// The write `op` makes, if the card shows what it changes.
    pub(crate) const fn of(op: &Op) -> Option<Self> {
        match *op {
            Op::PutSettings { base, changes } => Some(Self::Settings { base, changes }),
            Op::SetLock { locked } => Some(Self::Lock { locked }),
            Op::PutPlans { base, plans } => Some(Self::Plans { base, plans }),
            _ => None,
        }
    }

    /// Put the write on `facts` as `answer` landed it; whether the card moved.
    pub(crate) fn landed(&self, facts: &mut Facts, answer: Option<&Answer>) -> bool {
        let Some(Answer::Done { generation }) = answer else {
            return false;
        };
        match *self {
            Self::Settings { base, changes } => {
                let (Some(settings), Some(generation)) = (facts.settings.as_mut(), *generation)
                else {
                    return false;
                };
                if settings.generation != base {
                    return false;
                }
                patch(settings, changes);
                settings.generation = generation;
                if let Some(status) = facts.status.as_mut() {
                    status.settings_generation = generation;
                }
                true
            }
            Self::Lock { locked } => {
                let Some(status) = facts.status.as_mut() else {
                    return false;
                };
                status.locked = Some(locked);
                true
            }
            Self::Plans { base, plans } => {
                let (Some(held), Some(generation)) = (facts.plans.as_mut(), *generation) else {
                    return false;
                };
                if held.generation != base {
                    return false;
                }
                held.plans = plans;
                held.generation = generation;
                true
            }
        }
    }
}

/// `settings` with `changes` applied, as the feeder applies a patch: what it
/// does not name stays.
fn patch(settings: &mut Settings, changes: SettingsPatch) {
    let SettingsPatch {
        lights,
        sound,
        volume,
        lights_window,
        sound_window,
        desiccant_days,
        meal_reminder,
    } = changes;
    if let Some(lights) = lights {
        settings.lights = lights;
    }
    if let Some(sound) = sound {
        settings.sound = sound;
    }
    if let Some(volume) = volume {
        settings.volume = volume;
    }
    if let Some(window) = lights_window {
        settings.lights_window = window;
    }
    if let Some(window) = sound_window {
        settings.sound_window = window;
    }
    if let Some(days) = desiccant_days {
        settings.desiccant_days = days;
    }
    if let Some(reminder) = meal_reminder {
        settings.meal_reminder = reminder;
    }
}

/// `later` over `earlier`, field by field.
fn merged(earlier: SettingsPatch, later: SettingsPatch) -> SettingsPatch {
    SettingsPatch {
        lights: later.lights.or(earlier.lights),
        sound: later.sound.or(earlier.sound),
        volume: later.volume.or(earlier.volume),
        lights_window: later.lights_window.or(earlier.lights_window),
        sound_window: later.sound_window.or(earlier.sound_window),
        desiccant_days: later.desiccant_days.or(earlier.desiccant_days),
        meal_reminder: later.meal_reminder.or(earlier.meal_reminder),
    }
}

/// A settings write to send: its op, and everyone its one answer goes to.
pub(crate) struct Outgoing {
    pub(crate) op: Op,
    pub(crate) write: Write,
    pub(crate) replies: Vec<Reply>,
}

impl Outgoing {
    fn settings(base: Generation, changes: SettingsPatch, replies: Vec<Reply>) -> Self {
        Self {
            op: Op::PutSettings { base, changes },
            write: Write::Settings { base, changes },
            replies,
        }
    }
}

/// What follows a settings write's answer.
pub(crate) enum Next {
    /// Nothing waited.
    Idle,
    /// The writes that waited, merged, on the generation just answered.
    Send(Outgoing),
    /// The writes that waited, which never went: the base they would go on
    /// is not known.
    Unsent(Vec<Reply>),
}

/// Settings writes one at a time.
#[derive(Default)]
pub(crate) struct SettingsQueue {
    /// A settings write is out, unanswered.
    out: bool,
    /// What was asked since, merged, and who asked it.
    waiting: Option<(SettingsPatch, Vec<Reply>)>,
}

impl SettingsQueue {
    /// A settings write from a screen: the one to send now, or `None` when it
    /// waits behind the one out.
    pub(crate) fn ask(
        &mut self,
        base: Generation,
        changes: SettingsPatch,
        reply: Reply,
    ) -> Option<Outgoing> {
        if !self.out {
            self.out = true;
            return Some(Outgoing::settings(base, changes, vec![reply]));
        }
        let (held, replies) = self
            .waiting
            .get_or_insert_with(|| (SettingsPatch::default(), Vec::new()));
        *held = merged(*held, changes);
        replies.push(reply);
        None
    }

    /// The write out was answered with `answer`, or `None` with none.
    pub(crate) fn answered(&mut self, answer: Option<&Answer>) -> Next {
        let Some((changes, replies)) = self.waiting.take() else {
            self.out = false;
            return Next::Idle;
        };
        match answer {
            Some(Answer::Done {
                generation: Some(generation),
            }) => Next::Send(Outgoing::settings(*generation, changes, replies)),
            _ => {
                self.out = false;
                Next::Unsent(replies)
            }
        }
    }

    /// The writes still waiting, which never went, as the session ends.
    pub(crate) fn take(&mut self) -> Vec<Reply> {
        self.waiting
            .take()
            .map(|(_, replies)| replies)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::{MealReminder, Refusal, Weekdays, Window};

    use super::*;

    fn settings() -> Settings {
        Settings {
            generation: Generation(7),
            lights: true,
            sound: true,
            volume: 50,
            lights_window: Window::Always,
            sound_window: Window::Always,
            desiccant_days: 30,
            desiccant_replaced: None,
            meal_reminder: MealReminder::Off,
            writable: true,
        }
    }

    fn volume(volume: u8) -> SettingsPatch {
        SettingsPatch {
            volume: Some(volume),
            ..SettingsPatch::default()
        }
    }

    fn done(generation: u32) -> Answer {
        Answer::Done {
            generation: Some(Generation(generation)),
        }
    }

    fn reply() -> (Reply, oneshot::Receiver<Heard>) {
        oneshot::channel()
    }

    fn plans() -> wellfed_model::Plans {
        wellfed_model::Plans {
            generation: Generation(3),
            writable: true,
            plans: [Some(Plan {
                minute: 7 * 60,
                portions: 1,
                days: Weekdays::from_bits(0x7f),
                enabled: true,
                call: 0,
            }); PLANS],
        }
    }

    #[test]
    fn a_done_puts_the_settings_and_their_generation_on_the_card_at_once() {
        let mut facts = Facts {
            settings: Some(settings()),
            ..Facts::default()
        };
        let write = Write::of(&Op::PutSettings {
            base: Generation(7),
            changes: SettingsPatch {
                sound: Some(false),
                volume: Some(20),
                ..SettingsPatch::default()
            },
        })
        .expect("a settings write");

        assert!(write.landed(&mut facts, Some(&done(8))));

        let landed = facts.settings.expect("still read");
        assert_eq!(landed.generation, Generation(8));
        assert_eq!(landed.volume, 20);
        assert!(!landed.sound);
        assert!(landed.lights, "what the patch does not name stays");
    }

    #[test]
    fn a_refusal_no_answer_or_a_card_on_another_generation_moves_nothing() {
        let write = Write::Settings {
            base: Generation(7),
            changes: volume(20),
        };
        let mut facts = Facts {
            settings: Some(settings()),
            ..Facts::default()
        };

        let stale = Answer::Refused(Refusal::Stale {
            current: Generation(9),
        });
        assert!(!write.landed(&mut facts, Some(&stale)));
        assert!(!write.landed(&mut facts, None));
        assert_eq!(facts.settings, Some(settings()));

        // The read that the change's event brings came first: it is newer.
        let read = Settings {
            generation: Generation(9),
            volume: 80,
            ..settings()
        };
        facts.settings = Some(read);
        assert!(!write.landed(&mut facts, Some(&done(8))));
        assert_eq!(facts.settings, Some(read));
    }

    #[test]
    fn a_done_puts_the_plans_and_their_generation_on_the_card_at_once() {
        let mut facts = Facts {
            plans: Some(plans()),
            ..Facts::default()
        };
        let mut table = plans().plans;
        if let Some(Some(plan)) = table.first_mut() {
            plan.enabled = false;
        }
        let write = Write::of(&Op::PutPlans {
            base: Generation(3),
            plans: table,
        })
        .expect("a plans write");

        assert!(write.landed(&mut facts, Some(&done(4))));

        let landed = facts.plans.expect("still read");
        assert_eq!(landed.generation, Generation(4));
        assert_eq!(landed.plans[0].map(|plan| plan.enabled), Some(false));
    }

    #[test]
    fn only_settings_plans_and_the_lock_are_taken_at_once() {
        assert_eq!(
            Write::of(&Op::SetLock { locked: true }),
            Some(Write::Lock { locked: true })
        );
        assert_eq!(Write::of(&Op::GetSettings), None);
        assert_eq!(Write::of(&Op::ResetDesiccant), None);
        assert_eq!(Write::of(&Op::Feed { portions: 1 }), None);
    }

    #[test]
    fn a_second_settings_write_waits_and_goes_on_the_first_s_generation() {
        let mut queue = SettingsQueue::default();
        let (first, _first) = reply();
        let (second, _second) = reply();

        let sent = queue
            .ask(Generation(7), volume(20), first)
            .expect("nothing is out, so it goes");
        assert_eq!(
            sent.op,
            Op::PutSettings {
                base: Generation(7),
                changes: volume(20)
            }
        );
        assert!(queue.ask(Generation(7), volume(40), second).is_none());

        let Next::Send(next) = queue.answered(Some(&done(8))) else {
            panic!("the second goes after the first's done");
        };
        assert_eq!(
            next.op,
            Op::PutSettings {
                base: Generation(8),
                changes: volume(40)
            }
        );
        assert_eq!(next.replies.len(), 1);
        assert!(matches!(queue.answered(Some(&done(9))), Next::Idle));
        assert!(queue.ask(Generation(9), volume(60), reply().0).is_some());
    }

    #[test]
    fn writes_that_wait_merge_field_by_field_and_the_later_wins() {
        let mut queue = SettingsQueue::default();
        assert!(queue.ask(Generation(7), volume(20), reply().0).is_some());
        let lights = SettingsPatch {
            lights: Some(false),
            volume: Some(30),
            ..SettingsPatch::default()
        };
        assert!(queue.ask(Generation(7), lights, reply().0).is_none());
        assert!(queue.ask(Generation(7), volume(45), reply().0).is_none());

        let Next::Send(next) = queue.answered(Some(&done(8))) else {
            panic!("the merged write goes");
        };
        assert_eq!(
            next.op,
            Op::PutSettings {
                base: Generation(8),
                changes: SettingsPatch {
                    lights: Some(false),
                    volume: Some(45),
                    ..SettingsPatch::default()
                }
            }
        );
        assert_eq!(next.replies.len(), 2, "one answer for each who asked");
    }

    #[test]
    fn a_refused_or_unanswered_write_ends_the_waiting_ones_unsent() {
        let refused = Answer::Refused(Refusal::Stale {
            current: Generation(9),
        });
        for answer in [Some(&refused), None] {
            let mut queue = SettingsQueue::default();
            assert!(queue.ask(Generation(7), volume(20), reply().0).is_some());
            assert!(queue.ask(Generation(7), volume(40), reply().0).is_none());

            let Next::Unsent(replies) = queue.answered(answer) else {
                panic!("nothing goes on a base it cannot trust");
            };
            assert_eq!(replies.len(), 1);

            // The queue is free again, and the next write goes at once.
            assert!(queue.ask(Generation(9), volume(40), reply().0).is_some());
        }
    }

    #[test]
    fn a_session_that_ends_hands_back_the_writes_still_waiting() {
        let mut queue = SettingsQueue::default();
        assert!(queue.ask(Generation(7), volume(20), reply().0).is_some());
        assert!(queue.ask(Generation(7), volume(40), reply().0).is_none());

        assert_eq!(queue.take().len(), 1);
        assert!(queue.take().is_empty());
    }
}
