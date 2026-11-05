//! The writes a screen makes with a switch or a slider, and how the session
//! carries them so a control moves on the feeder's first answer.
//!
//! A write's `Done` is the feeder saying it took the change, so the card takes
//! it at once rather than after the event and the read it brings: that read
//! still comes, and only confirms. The card takes it only when the card holds
//! the generation the write went on and the `Done` names the one after it,
//! which is what the feeder's stores number a write they made. A `Done` that
//! names anything else (a write the writer could not make leaves the
//! generation as it was) moves nothing, and the read shows what is true.
//!
//! Settings writes go out one at a time, a desiccant reset among them, since
//! it moves the settings generation too. One asked while another is out waits,
//! merged field by field with any others waiting (the later wins), and goes on
//! the generation the one out landed with, so a quick second change is not
//! refused as stale. A waiting write goes only after a `Done` that landed: one
//! out that was refused, never answered or answered with another generation
//! leaves no generation to trust, so the waiting ones end unsent. A change read
//! from another generation than the one out went on never waits: merged onto
//! the new one, its check against a stale read would be skipped.
//!
//! Plans writes go one at a time too. A meal switched while one is out waits,
//! and goes again as that switch on the table the one out made; any other
//! plans write asked meanwhile ends unsent. Nothing that went out is ever
//! sent again.

use tokio::sync::oneshot;
use wellfed_client::plans::{self, Edit};
use wellfed_model::{Answer, FeederSettingsPatch, Generation, Op, PLANS, Plan, Plans, Status};

use crate::session::Heard;
use crate::today::Facts;

/// Where one caller hears how its request ended.
pub(crate) type Reply = oneshot::Sender<Heard>;

/// The generation a write on `base` makes: settings and plans number each
/// write the store makes one past the last.
const fn after(base: Generation) -> Generation {
    Generation(base.0.wrapping_add(1))
}

/// Whether `answer` is a `Done` that names what a write on `base` makes.
fn landed_on(base: Generation, answer: Option<&Answer>) -> bool {
    matches!(answer, Some(Answer::Done { generation: Some(generation) }) if *generation == after(base))
}

/// A write whose `Done` the card takes as soon as it comes, or that waits in
/// the settings queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Settings {
        base: Generation,
        changes: FeederSettingsPatch,
    },
    /// The desiccant changed today, on the settings generation `base` when
    /// it is known. Its `Done` moves the card's settings generation, so a
    /// change tapped next goes on it; the date shows from the read its event
    /// brings.
    Reset {
        base: Option<Generation>,
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
    /// The write `op` makes, if the card shows what it changes or it moves
    /// the settings generation.
    pub(crate) const fn of(op: &Op) -> Option<Self> {
        match *op {
            Op::PutSettings { base, changes } => Some(Self::Settings { base, changes }),
            // The settings queue gives it the generation it goes on.
            Op::ResetDesiccant => Some(Self::Reset { base: None }),
            Op::SetLock { locked } => Some(Self::Lock { locked }),
            Op::PutPlans { base, plans } => Some(Self::Plans { base, plans }),
            _ => None,
        }
    }

    /// The queue this goes through, if any.
    pub(crate) const fn queue(&self) -> Option<Line> {
        match self {
            Self::Settings { .. } | Self::Reset { .. } => Some(Line::Settings),
            Self::Plans { .. } => Some(Line::Plans),
            Self::Lock { .. } => None,
        }
    }

    /// Put the write on `facts` as `answer` landed it; whether the card moved.
    pub(crate) fn landed(&self, facts: &mut Facts, answer: Option<&Answer>) -> bool {
        match *self {
            Self::Settings { base, changes } => {
                let Some(settings) = facts.settings.as_mut() else {
                    return false;
                };
                if settings.generation != base || !landed_on(base, answer) {
                    return false;
                }
                settings.apply(changes);
                settings.generation = after(base);
                if let Some(status) = facts.status.as_mut().and_then(Status::feeder_mut) {
                    status.settings_generation = after(base);
                }
                true
            }
            Self::Reset { base } => {
                let (Some(base), Some(settings)) = (base, facts.settings.as_mut()) else {
                    return false;
                };
                if settings.generation != base || !landed_on(base, answer) {
                    return false;
                }
                settings.generation = after(base);
                if let Some(status) = facts.status.as_mut().and_then(Status::feeder_mut) {
                    status.settings_generation = after(base);
                }
                true
            }
            Self::Lock { locked } => {
                let (Some(status), Some(Answer::Done { .. })) =
                    (facts.status.as_mut().and_then(Status::feeder_mut), answer)
                else {
                    return false;
                };
                status.locked = Some(locked);
                true
            }
            Self::Plans { base, plans } => {
                let Some(held) = facts.plans.as_mut() else {
                    return false;
                };
                if held.generation != base || !landed_on(base, answer) {
                    return false;
                }
                held.plans = plans;
                held.generation = after(base);
                true
            }
        }
    }
}

/// `later` over `earlier`, field by field.
fn merged(earlier: FeederSettingsPatch, later: FeederSettingsPatch) -> FeederSettingsPatch {
    FeederSettingsPatch {
        lights: later.lights.or(earlier.lights),
        sound: later.sound.or(earlier.sound),
        volume: later.volume.or(earlier.volume),
        lights_window: later.lights_window.or(earlier.lights_window),
        sound_window: later.sound_window.or(earlier.sound_window),
        desiccant_days: later.desiccant_days.or(earlier.desiccant_days),
        meal_reminder: later.meal_reminder.or(earlier.meal_reminder),
    }
}

/// A queued write to send: its op, and everyone its one answer goes to.
pub(crate) struct Outgoing {
    pub(crate) op: Op,
    pub(crate) write: Write,
    pub(crate) replies: Vec<Reply>,
}

impl Outgoing {
    fn settings(base: Generation, changes: FeederSettingsPatch, replies: Vec<Reply>) -> Self {
        Self {
            op: Op::PutSettings { base, changes },
            write: Write::Settings { base, changes },
            replies,
        }
    }

    const fn reset(base: Option<Generation>, replies: Vec<Reply>) -> Self {
        Self {
            op: Op::ResetDesiccant,
            write: Write::Reset { base },
            replies,
        }
    }

    const fn plans(base: Generation, plans: [Option<Plan>; PLANS], replies: Vec<Reply>) -> Self {
        Self {
            op: Op::PutPlans { base, plans },
            write: Write::Plans { base, plans },
            replies,
        }
    }
}

/// The queues writes wait in: one for each store whose generation they share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Line {
    Settings,
    Plans,
}

/// What became of a queued write a screen asked for.
pub(crate) enum Queued {
    /// Nothing was out: send it now.
    Send(Outgoing),
    /// It waits behind the write out.
    Waits,
    /// It was read from another generation than the one out went on, so it
    /// never goes.
    Held(Reply),
}

/// What follows a queued write's answer.
pub(crate) enum Next {
    /// Nothing waited.
    Idle,
    /// A write that waited, on the generation just landed.
    Send(Outgoing),
    /// The writes that waited, which never went: the base they would go on
    /// is not known.
    Unsent(Vec<Reply>),
}

/// The settings write out, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Out {
    #[default]
    Idle,
    /// On this generation.
    On(Generation),
    /// A reset sent before the settings were read: what it lands on is not
    /// known.
    Unknown,
}

/// Settings writes one at a time.
#[derive(Default)]
pub(crate) struct SettingsQueue {
    out: Out,
    /// The changes asked since, merged, and who asked them.
    changes: Option<(FeederSettingsPatch, Vec<Reply>)>,
    /// The resets asked since: one goes for them all.
    resets: Vec<Reply>,
}

impl SettingsQueue {
    /// A settings write from a screen. `write` is a settings change or a reset;
    /// `known` is the settings generation the card holds, which a reset goes on.
    pub(crate) fn ask(&mut self, write: Write, known: Option<Generation>, reply: Reply) -> Queued {
        if self.out == Out::Idle {
            let (out, outgoing) = match write {
                Write::Settings { base, changes } => (
                    Out::On(base),
                    Outgoing::settings(base, changes, vec![reply]),
                ),
                _ => (
                    known.map_or(Out::Unknown, Out::On),
                    Outgoing::reset(known, vec![reply]),
                ),
            };
            self.out = out;
            return Queued::Send(outgoing);
        }

        match write {
            Write::Settings { base, changes } if self.out == Out::On(base) => {
                let (held, replies) = self
                    .changes
                    .get_or_insert_with(|| (FeederSettingsPatch::default(), Vec::new()));
                *held = merged(*held, changes);
                replies.push(reply);
                Queued::Waits
            }
            Write::Settings { .. } => Queued::Held(reply),
            _ => {
                self.resets.push(reply);
                Queued::Waits
            }
        }
    }

    /// The write out was answered with `answer`, or `None` with none.
    pub(crate) fn answered(&mut self, answer: Option<&Answer>) -> Next {
        let landed = match self.out {
            Out::On(base) if landed_on(base, answer) => Some(after(base)),
            _ => None,
        };
        self.out = Out::Idle;

        let Some(base) = landed else {
            let unsent = self.take();
            return if unsent.is_empty() {
                Next::Idle
            } else {
                Next::Unsent(unsent)
            };
        };
        if let Some((changes, replies)) = self.changes.take() {
            self.out = Out::On(base);
            return Next::Send(Outgoing::settings(base, changes, replies));
        }
        if !self.resets.is_empty() {
            self.out = Out::On(base);
            return Next::Send(Outgoing::reset(
                Some(base),
                std::mem::take(&mut self.resets),
            ));
        }
        Next::Idle
    }

    /// The writes still waiting, which never went.
    pub(crate) fn take(&mut self) -> Vec<Reply> {
        let mut waiting = self
            .changes
            .take()
            .map(|(_, replies)| replies)
            .unwrap_or_default();
        waiting.append(&mut self.resets);
        waiting
    }
}

/// A meal switched on or off: a plans write that can go again on the table
/// the one before it made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MealSwitch {
    pub(crate) slot: u8,
    pub(crate) enabled: bool,
}

/// Plans writes one at a time. A plans write sends the whole table, so one
/// that waits goes again only as what it is: a meal switched, put on the
/// table the one out made. Anything else (the editor's whole meal) ends
/// unsent, since the table it was made from is gone.
#[derive(Default)]
pub(crate) struct PlansQueue {
    /// The write out: the revision it went on and the table it sends.
    out: Option<(Generation, [Option<Plan>; PLANS])>,
    /// The meals switched since, in order, and who asked each.
    waiting: Vec<(MealSwitch, Reply)>,
}

impl PlansQueue {
    /// A plans write from a screen: `plans` on `base`, and the meal switch it
    /// is, if it is one.
    pub(crate) fn ask(
        &mut self,
        base: Generation,
        plans: [Option<Plan>; PLANS],
        switch: Option<MealSwitch>,
        reply: Reply,
    ) -> Queued {
        let Some((on, _)) = self.out else {
            self.out = Some((base, plans));
            return Queued::Send(Outgoing::plans(base, plans, vec![reply]));
        };
        match switch {
            Some(switch) if on == base => {
                self.waiting.push((switch, reply));
                Queued::Waits
            }
            _ => Queued::Held(reply),
        }
    }

    /// The write out was answered with `answer`, or `None` with none. A meal
    /// switched since whose meal the new table no longer holds hears it was
    /// held, here.
    pub(crate) fn answered(&mut self, answer: Option<&Answer>) -> Next {
        let out = self.out.take();
        let landed = out.filter(|(base, _)| landed_on(*base, answer));
        let Some((base, table)) = landed else {
            let unsent = self.take();
            return if unsent.is_empty() {
                Next::Idle
            } else {
                Next::Unsent(unsent)
            };
        };
        if self.waiting.is_empty() {
            return Next::Idle;
        }

        let mut next = Plans {
            generation: after(base),
            writable: true,
            plans: table,
        };
        let mut replies = Vec::new();
        for (switch, reply) in self.waiting.drain(..) {
            let edit = Edit::Enabled {
                slot: switch.slot,
                enabled: switch.enabled,
            };
            match plans::edited(&next, edit) {
                Ok(table) => {
                    next.plans = table;
                    replies.push(reply);
                }
                Err(_) => {
                    // Dart stopped waiting: nothing to tell it.
                    let _ = reply.send(Heard::Held);
                }
            }
        }
        if replies.is_empty() {
            return Next::Idle;
        }
        self.out = Some((next.generation, next.plans));
        Next::Send(Outgoing::plans(next.generation, next.plans, replies))
    }

    /// The writes still waiting, which never went.
    pub(crate) fn take(&mut self) -> Vec<Reply> {
        self.waiting.drain(..).map(|(_, reply)| reply).collect()
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::{FeederSettings, MealReminder, Refusal, Weekdays, Window};

    use super::*;

    fn settings() -> FeederSettings {
        FeederSettings {
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

    fn volume(volume: u8) -> FeederSettingsPatch {
        FeederSettingsPatch {
            volume: Some(volume),
            ..FeederSettingsPatch::default()
        }
    }

    fn change(base: u32, changes: FeederSettingsPatch) -> Write {
        Write::Settings {
            base: Generation(base),
            changes,
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

    fn sent(queued: Queued) -> Outgoing {
        let Queued::Send(outgoing) = queued else {
            panic!("nothing is out, so it goes");
        };
        outgoing
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
            changes: FeederSettingsPatch {
                sound: Some(false),
                volume: Some(20),
                ..FeederSettingsPatch::default()
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
        let write = change(7, volume(20));
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
        let read = FeederSettings {
            generation: Generation(9),
            volume: 80,
            ..settings()
        };
        facts.settings = Some(read);
        assert!(!write.landed(&mut facts, Some(&done(8))));
        assert_eq!(facts.settings, Some(read));
    }

    /// A write the writer could not make answers with the generation as it
    /// was, and one past another client's write names a later one: neither is
    /// what this write made, so the read shows what is true.
    #[test]
    fn a_done_on_any_generation_but_the_next_moves_nothing() {
        let write = change(7, volume(20));
        let mut facts = Facts {
            settings: Some(settings()),
            plans: Some(plans()),
            ..Facts::default()
        };

        assert!(!write.landed(&mut facts, Some(&done(7))));
        assert!(!write.landed(&mut facts, Some(&done(9))));
        assert!(!write.landed(&mut facts, Some(&Answer::Done { generation: None })));
        assert_eq!(facts.settings, Some(settings()));

        let plans_write = Write::Plans {
            base: Generation(3),
            plans: [None; PLANS],
        };
        assert!(!plans_write.landed(&mut facts, Some(&done(3))));
        assert_eq!(facts.plans, Some(plans()));
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

    /// A change tapped after a reset goes on the generation the reset made,
    /// so the card takes that much of it at once.
    #[test]
    fn a_reset_queues_with_the_settings_and_moves_only_their_generation() {
        let mut facts = Facts {
            settings: Some(settings()),
            ..Facts::default()
        };
        assert_eq!(
            Write::of(&Op::ResetDesiccant),
            Some(Write::Reset { base: None })
        );
        assert_eq!(Write::Reset { base: None }.queue(), Some(Line::Settings));

        let unknown = Write::Reset { base: None };
        assert!(!unknown.landed(&mut facts, Some(&done(8))));
        let reset = Write::Reset {
            base: Some(Generation(7)),
        };
        assert!(!reset.landed(&mut facts, Some(&done(7))));
        assert_eq!(facts.settings, Some(settings()));

        assert!(reset.landed(&mut facts, Some(&done(8))));
        assert_eq!(
            facts.settings,
            Some(FeederSettings {
                generation: Generation(8),
                ..settings()
            })
        );
    }

    #[test]
    fn only_settings_plans_the_lock_and_a_reset_are_writes() {
        assert_eq!(
            Write::of(&Op::SetLock { locked: true }),
            Some(Write::Lock { locked: true })
        );
        assert_eq!(Write::Lock { locked: true }.queue(), None);
        assert_eq!(
            Write::of(&Op::PutPlans {
                base: Generation(3),
                plans: [None; PLANS]
            })
            .and_then(|write| write.queue()),
            Some(Line::Plans)
        );
        assert_eq!(Write::of(&Op::GetSettings), None);
        assert_eq!(Write::of(&Op::Feed { portions: 1 }), None);
    }

    #[test]
    fn a_second_settings_write_waits_and_goes_on_the_first_s_generation() {
        let mut queue = SettingsQueue::default();

        let first = sent(queue.ask(change(7, volume(20)), Some(Generation(7)), reply().0));
        assert_eq!(
            first.op,
            Op::PutSettings {
                base: Generation(7),
                changes: volume(20)
            }
        );
        assert!(matches!(
            queue.ask(change(7, volume(40)), Some(Generation(7)), reply().0),
            Queued::Waits
        ));

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
        assert!(matches!(
            queue.ask(change(9, volume(60)), Some(Generation(9)), reply().0),
            Queued::Send(_)
        ));
    }

    #[test]
    fn writes_that_wait_merge_field_by_field_and_the_later_wins() {
        let mut queue = SettingsQueue::default();
        sent(queue.ask(change(7, volume(20)), None, reply().0));
        let lights = FeederSettingsPatch {
            lights: Some(false),
            volume: Some(30),
            ..FeederSettingsPatch::default()
        };
        assert!(matches!(
            queue.ask(change(7, lights), None, reply().0),
            Queued::Waits
        ));
        assert!(matches!(
            queue.ask(change(7, volume(45)), None, reply().0),
            Queued::Waits
        ));

        let Next::Send(next) = queue.answered(Some(&done(8))) else {
            panic!("the merged write goes");
        };
        assert_eq!(
            next.op,
            Op::PutSettings {
                base: Generation(8),
                changes: FeederSettingsPatch {
                    lights: Some(false),
                    volume: Some(45),
                    ..FeederSettingsPatch::default()
                }
            }
        );
        assert_eq!(next.replies.len(), 2, "one answer for each who asked");
    }

    /// Merged onto the new generation, a change read from another would skip
    /// its check against a stale read.
    #[test]
    fn a_change_read_from_another_generation_never_waits() {
        let mut queue = SettingsQueue::default();
        sent(queue.ask(change(7, volume(20)), None, reply().0));

        assert!(matches!(
            queue.ask(change(5, volume(40)), None, reply().0),
            Queued::Held(_)
        ));
        assert!(matches!(queue.answered(Some(&done(8))), Next::Idle));
    }

    #[test]
    fn a_refused_unanswered_or_unlanded_write_ends_the_waiting_ones_unsent() {
        let refused = Answer::Refused(Refusal::Stale {
            current: Generation(9),
        });
        // The writer could not make it, or another write came between.
        let unchanged = done(7);
        let another = done(9);
        for answer in [Some(&refused), None, Some(&unchanged), Some(&another)] {
            let mut queue = SettingsQueue::default();
            sent(queue.ask(change(7, volume(20)), None, reply().0));
            assert!(matches!(
                queue.ask(change(7, volume(40)), None, reply().0),
                Queued::Waits
            ));
            assert!(matches!(
                queue.ask(Write::Reset { base: None }, None, reply().0),
                Queued::Waits
            ));

            let Next::Unsent(replies) = queue.answered(answer) else {
                panic!("nothing goes on a base it cannot trust");
            };
            assert_eq!(replies.len(), 2);

            // The queue is free again, and the next write goes at once.
            sent(queue.ask(change(9, volume(40)), None, reply().0));
        }
    }

    /// A reset moves the settings generation, so a change behind it goes on
    /// the generation the reset made, and one behind a change likewise.
    #[test]
    fn a_reset_waits_and_is_waited_behind_like_any_settings_write() {
        let mut queue = SettingsQueue::default();
        let reset = sent(queue.ask(Write::Reset { base: None }, Some(Generation(7)), reply().0));
        assert_eq!(reset.op, Op::ResetDesiccant);
        assert!(matches!(
            queue.ask(change(7, volume(40)), Some(Generation(7)), reply().0),
            Queued::Waits
        ));
        assert!(matches!(
            queue.ask(Write::Reset { base: None }, Some(Generation(7)), reply().0),
            Queued::Waits
        ));

        let Next::Send(change_next) = queue.answered(Some(&done(8))) else {
            panic!("the change goes on the reset's generation");
        };
        assert_eq!(
            change_next.op,
            Op::PutSettings {
                base: Generation(8),
                changes: volume(40)
            }
        );
        let Next::Send(reset_next) = queue.answered(Some(&done(9))) else {
            panic!("the reset that waited goes next");
        };
        assert_eq!(reset_next.op, Op::ResetDesiccant);
        assert!(matches!(queue.answered(Some(&done(10))), Next::Idle));
    }

    /// A reset sent before the settings were read lands on a generation
    /// nobody knows, so what waits behind it never goes.
    #[test]
    fn what_waits_behind_a_reset_on_an_unknown_generation_never_goes() {
        let mut queue = SettingsQueue::default();
        sent(queue.ask(Write::Reset { base: None }, None, reply().0));
        assert!(matches!(
            queue.ask(Write::Reset { base: None }, None, reply().0),
            Queued::Waits
        ));

        assert!(matches!(queue.answered(Some(&done(8))), Next::Unsent(_)));
    }

    #[test]
    fn a_session_that_ends_hands_back_the_writes_still_waiting() {
        let mut queue = SettingsQueue::default();
        sent(queue.ask(change(7, volume(20)), None, reply().0));
        assert!(matches!(
            queue.ask(change(7, volume(40)), None, reply().0),
            Queued::Waits
        ));

        assert_eq!(queue.take().len(), 1);
        assert!(queue.take().is_empty());
    }

    fn switch(slot: u8, enabled: bool) -> Option<MealSwitch> {
        Some(MealSwitch { slot, enabled })
    }

    fn switched(slot: usize, enabled: bool) -> [Option<Plan>; PLANS] {
        let mut table = plans().plans;
        if let Some(Some(plan)) = table.get_mut(slot) {
            plan.enabled = enabled;
        }
        table
    }

    fn sent_plans(next: Next) -> Outgoing {
        let Next::Send(outgoing) = next else {
            panic!("the meal switched since goes");
        };
        outgoing
    }

    /// Two meals switched within one round trip: the second goes as its
    /// switch on the table the first made, so both land.
    #[test]
    fn a_meal_switched_while_another_is_out_goes_on_the_table_it_made() {
        let mut queue = PlansQueue::default();
        let first = switched(0, false);

        let out = sent(queue.ask(Generation(3), first, switch(0, false), reply().0));
        assert_eq!(
            out.op,
            Op::PutPlans {
                base: Generation(3),
                plans: first
            }
        );
        // Made from the table as the screen read it: without the first.
        assert!(matches!(
            queue.ask(
                Generation(3),
                switched(1, false),
                switch(1, false),
                reply().0
            ),
            Queued::Waits
        ));
        assert!(matches!(
            queue.ask(
                Generation(3),
                switched(2, false),
                switch(2, false),
                reply().0
            ),
            Queued::Waits
        ));

        let next = sent_plans(queue.answered(Some(&done(4))));
        let mut both = first;
        for slot in [1, 2] {
            if let Some(Some(plan)) = both.get_mut(slot) {
                plan.enabled = false;
            }
        }
        assert_eq!(
            next.op,
            Op::PutPlans {
                base: Generation(4),
                plans: both
            }
        );
        assert_eq!(next.replies.len(), 2);
        assert!(matches!(queue.answered(Some(&done(5))), Next::Idle));
    }

    /// The editor's whole meal was made from a table that is gone, and a
    /// switch read from another revision would skip its check against a stale
    /// read: neither waits.
    #[test]
    fn a_whole_table_write_or_one_from_another_revision_never_waits() {
        let mut queue = PlansQueue::default();
        sent(queue.ask(
            Generation(3),
            switched(0, false),
            switch(0, false),
            reply().0,
        ));

        assert!(matches!(
            queue.ask(Generation(3), switched(1, false), None, reply().0),
            Queued::Held(_)
        ));
        assert!(matches!(
            queue.ask(
                Generation(2),
                switched(1, false),
                switch(1, false),
                reply().0
            ),
            Queued::Held(_)
        ));
        assert!(matches!(queue.answered(Some(&done(4))), Next::Idle));
    }

    #[test]
    fn a_refused_unanswered_or_unlanded_plans_write_ends_the_waiting_ones_unsent() {
        let refused = Answer::Refused(Refusal::Busy);
        let unchanged = done(3);
        for answer in [Some(&refused), None, Some(&unchanged)] {
            let mut queue = PlansQueue::default();
            sent(queue.ask(
                Generation(3),
                switched(0, false),
                switch(0, false),
                reply().0,
            ));
            assert!(matches!(
                queue.ask(
                    Generation(3),
                    switched(1, false),
                    switch(1, false),
                    reply().0
                ),
                Queued::Waits
            ));

            let Next::Unsent(replies) = queue.answered(answer) else {
                panic!("nothing goes on a table it cannot trust");
            };
            assert_eq!(replies.len(), 1);
            sent(queue.ask(Generation(5), switched(0, true), switch(0, true), reply().0));
        }
    }

    /// A meal the new table no longer holds cannot be switched on it.
    #[test]
    fn a_switch_whose_meal_is_gone_ends_held() {
        let mut queue = PlansQueue::default();
        let mut gone = plans().plans;
        if let Some(slot) = gone.get_mut(1) {
            *slot = None;
        }
        sent(queue.ask(Generation(3), gone, None, reply().0));
        let (waiting, mut heard) = reply();
        assert!(matches!(
            queue.ask(Generation(3), switched(1, false), switch(1, false), waiting),
            Queued::Waits
        ));

        assert!(matches!(queue.answered(Some(&done(4))), Next::Idle));
        assert!(matches!(heard.try_recv(), Ok(Heard::Held)));
    }
}
