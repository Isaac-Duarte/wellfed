//! The admission rules against synthetic gestures and requests: setup
//! takes its code from the buttons in time, a press confirms a signer offer,
//! invites are single-use, expire
//! and never grant more than their issuer holds, and the recovery window
//! needs an unlocked panel and admits one owner.

use wellfed_core::buttons::{Button, Buttons, Gesture};
use wellfed_device_api::clients::Role;
use wellfed_device_api::mailbox::Lock;
use wellfed_device_api::pairing::{Config, Event, INVITES, Pairing, Path, Refusal};
use wellfed_session::setup_code::Code;

const CONFIRM: u32 = Config::DEFAULT.confirm_ms;
const INVITE: u32 = Config::DEFAULT.invite_ms;
const RECOVERY: u32 = Config::DEFAULT.recovery_ms;
/// The static key of the client that asks for an invite.
const ISSUER: [u8; 32] = [0x11; 32];
/// The static key a joining client redeems an invite with.
const JOINER: [u8; 32] = [0x22; 32];

fn press(button: Button) -> Gesture {
    Gesture::Down {
        chord: Buttons::of(button),
    }
}

/// A short press of `button` alone, as the button service reports it: its
/// `Down`, which a pending code takes, then its `Up`, whose event is the
/// answer (or the `Down`'s, if that already decided).
fn tap(p: &mut Pairing, now: u32, button: Button, lock: Lock) -> Option<Event> {
    let down = p.gesture(now, press(button), lock);
    if !matches!(down, Some(Event::Pressed { .. })) {
        return down;
    }
    let up = Gesture::Up {
        chord: Buttons::of(button),
        level: 0,
        held_ms: 100,
    };
    p.gesture(now, up, lock)
}

/// What the button service reports for "press Lock, then add `with`, and
/// hold": Lock goes down alone before the chord forms.
fn chord_hold(with: Button) -> [Gesture; 4] {
    let lock = Buttons::of(Button::Lock);
    let both = lock.with(with);
    [
        Gesture::Down { chord: lock },
        Gesture::Down { chord: both },
        Gesture::Hold {
            chord: both,
            level: 1,
        },
        Gesture::Up {
            chord: both,
            level: 1,
            held_ms: 5_200,
        },
    ]
}

fn recovery_hold() -> Gesture {
    Gesture::Hold {
        chord: Buttons::of(Button::Lock).with(Button::Dispense),
        level: 1,
    }
}

fn code() -> Code {
    Code::from_bits(0b00_01_10_11_11_10_01_00)
}

fn keyed(entered: u8) -> Option<Event> {
    Some(Event::Keyed {
        path: Path::Setup,
        entered,
    })
}

fn refused(path: Path, refusal: Refusal) -> Event {
    Event::Refused { path, refusal }
}

fn admit(path: Path, role: Role) -> Event {
    Event::Admit {
        path,
        role,
        inviter: None,
    }
}

/// An invite's admission, naming the key prefix of the issuer it came from.
fn invited(role: Role, issuer: [u8; 32]) -> Event {
    Event::Admit {
        path: Path::Invite,
        role,
        inviter: Some(wellfed_device_api::key_prefix(&issuer)),
    }
}

#[test]
fn the_configuration_is_valid() {
    assert!(Config::DEFAULT.valid());
    assert!(
        !Config {
            hold_level: 0,
            ..Config::DEFAULT
        }
        .valid()
    );
}

#[test]
fn setup_takes_its_code_from_the_buttons_even_when_locked() {
    let mut p = Pairing::new(Config::DEFAULT);
    assert_eq!(p.setup(0), Event::Confirming { path: Path::Setup });
    assert_eq!(p.setup(10), refused(Path::Setup, Refusal::Busy));

    let code = code();
    let (last, first) = code.presses().split_last().unwrap();
    for (i, &button) in first.iter().enumerate() {
        let entered = u8::try_from(i + 1).unwrap();
        assert_eq!(
            p.gesture(100, press(button), Lock::Locked),
            Some(Event::Pressed { path: Path::Setup }),
            "taken, so the panel never acts on it"
        );
        let up = Gesture::Up {
            chord: Buttons::of(button),
            level: 0,
            held_ms: 100,
        };
        assert_eq!(p.gesture(100, up, Lock::Locked), keyed(entered));
    }

    assert_eq!(
        tap(&mut p, CONFIRM - 1, *last, Lock::Locked),
        Some(Event::Entered {
            path: Path::Setup,
            code
        })
    );
    assert_eq!(
        p.gesture(CONFIRM, press(Button::Dispense), Lock::Locked),
        None,
        "nothing pending: the press goes to the panel"
    );
}

#[test]
fn a_lone_dispense_press_does_not_admit_setup() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.setup(0);
    assert_eq!(tap(&mut p, 100, Button::Dispense, Lock::Unlocked), keyed(1));
    assert_eq!(p.tick(200), None);
}

#[test]
fn an_unanswered_offer_expires() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.setup(0);
    assert_eq!(p.tick(CONFIRM - 1), None);
    assert_eq!(
        p.tick(CONFIRM),
        Some(refused(Path::Setup, Refusal::Unconfirmed))
    );
    assert_eq!(p.tick(CONFIRM + 1), None);

    // A press arriving after the deadline but before the tick still fails.
    p.setup(1_000);
    assert_eq!(tap(&mut p, 1_000, Button::Mic, Lock::Unlocked), keyed(1));
    assert_eq!(
        p.gesture(1_000 + CONFIRM, press(Button::Dispense), Lock::Unlocked),
        Some(refused(Path::Setup, Refusal::Unconfirmed))
    );
}

#[test]
fn only_an_owner_offers_a_signing_key() {
    let mut p = Pairing::new(Config::DEFAULT);
    assert_eq!(
        p.signer(0, Role::Member),
        refused(Path::Signer, Refusal::NotAllowed)
    );
    assert_eq!(
        p.signer(0, Role::Owner),
        Event::Confirming { path: Path::Signer }
    );
    assert_eq!(
        p.gesture(5, press(Button::Dispense), Lock::Unlocked),
        Some(Event::Pressed { path: Path::Signer })
    );
    let up = Gesture::Up {
        chord: Buttons::of(Button::Dispense),
        level: 0,
        held_ms: 100,
    };
    assert_eq!(
        p.gesture(105, up, Lock::Unlocked),
        Some(admit(Path::Signer, Role::UpdateSigner))
    );
}

/// An owner whose recovery hold starts with Dispense confirms no update
/// key: the press grew into the chord, and the hold opens recovery.
#[test]
fn a_recovery_hold_started_with_dispense_confirms_no_signer() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.signer(0, Role::Owner);
    let both = Buttons::of(Button::Dispense).with(Button::Lock);

    assert_eq!(
        p.gesture(10, press(Button::Dispense), Lock::Unlocked),
        Some(Event::Pressed { path: Path::Signer })
    );
    assert_eq!(
        p.gesture(60, Gesture::Down { chord: both }, Lock::Unlocked),
        None
    );
    assert_eq!(
        p.gesture(5_010, recovery_hold(), Lock::Unlocked),
        Some(Event::RecoveryOpened)
    );
    let up = Gesture::Up {
        chord: both,
        level: 1,
        held_ms: 5_200,
    };
    assert_eq!(
        p.gesture(5_210, up, Lock::Unlocked),
        Some(Event::Swallowed { path: Path::Signer })
    );

    // The offer still waits for a press of its own.
    assert_eq!(
        tap(&mut p, 6_000, Button::Dispense, Lock::Unlocked),
        Some(admit(Path::Signer, Role::UpdateSigner))
    );
}

#[test]
fn invites_are_single_use_and_bounded_by_the_issuer() {
    let mut p = Pairing::new(Config::DEFAULT);
    assert_eq!(
        p.invite(0, Role::Member, ISSUER, Role::Owner, None, [1; 32]),
        refused(Path::Invite, Refusal::NotAllowed)
    );
    assert_eq!(
        p.invite(0, Role::Server, ISSUER, Role::Member, None, [1; 32]),
        refused(Path::Invite, Refusal::NotAllowed)
    );
    assert_eq!(
        p.invite(0, Role::Owner, ISSUER, Role::UpdateSigner, None, [1; 32]),
        refused(Path::Invite, Refusal::NotAllowed)
    );

    let Event::Invited { id } = p.invite(0, Role::Member, ISSUER, Role::Member, None, [7; 32])
    else {
        panic!("a member invites a member");
    };
    assert_eq!(p.invite_secret(10, id), Some([7; 32]));
    assert_eq!(p.invite_secret(10, id + 1), None);
    assert_eq!(p.redeem(20, id, &JOINER), invited(Role::Member, ISSUER));
    assert_eq!(
        p.redeem(30, id, &JOINER),
        refused(Path::Invite, Refusal::Unknown)
    );
    assert_eq!(p.invite_secret(30, id), None);
}

/// An invite bound to a key admits that key alone. Another key holding
/// the secret is refused and the invite kept for the right one.
#[test]
fn a_bound_invite_admits_only_its_key() {
    let mut p = Pairing::new(Config::DEFAULT);
    let bound = Some(JOINER);
    let Event::Invited { id } = p.invite(0, Role::Member, ISSUER, Role::Member, bound, [7; 32])
    else {
        panic!("a member invites a member's key");
    };

    assert_eq!(
        p.redeem(10, id, &[0x33; 32]),
        refused(Path::Invite, Refusal::WrongKey)
    );
    assert_eq!(p.invite_secret(10, id), Some([7; 32]), "kept");
    assert_eq!(p.redeem(20, id, &JOINER), invited(Role::Member, ISSUER));
    assert_eq!(
        p.redeem(30, id, &JOINER),
        refused(Path::Invite, Refusal::Unknown)
    );
}

/// An invite dies with its issuer's key: once the issuer is revoked, the
/// invite it made is gone, and only another issuer's stays.
#[test]
fn an_invite_goes_with_its_issuers_key() {
    let mut p = Pairing::new(Config::DEFAULT);
    let Event::Invited { id: revoked } =
        p.invite(0, Role::Owner, ISSUER, Role::Owner, None, [7; 32])
    else {
        panic!("an owner invites an owner");
    };
    let other = [0x44; 32];
    let Event::Invited { id: kept } = p.invite(0, Role::Owner, other, Role::Member, None, [8; 32])
    else {
        panic!("another owner invites a member");
    };

    p.issuers_left(|key| *key == other);
    assert_eq!(p.invite_secret(10, revoked), None);
    assert_eq!(
        p.redeem(10, revoked, &JOINER),
        refused(Path::Invite, Refusal::Unknown)
    );
    assert_eq!(p.redeem(10, kept, &JOINER), invited(Role::Member, other));
}

#[test]
fn invites_expire_and_are_limited() {
    let mut p = Pairing::new(Config::DEFAULT);
    let ids: Vec<u32> = (0..INVITES)
        .map(
            |_| match p.invite(0, Role::Owner, ISSUER, Role::Server, None, [2; 32]) {
                Event::Invited { id } => id,
                other => panic!("{other:?}"),
            },
        )
        .collect();
    assert_eq!(
        p.invite(1, Role::Owner, ISSUER, Role::Member, None, [3; 32]),
        refused(Path::Invite, Refusal::Full)
    );
    assert_eq!(p.invite_secret(INVITE - 1, ids[0]), Some([2; 32]));
    assert_eq!(p.invite_secret(INVITE, ids[0]), None);
    assert_eq!(
        p.redeem(INVITE, ids[1], &JOINER),
        refused(Path::Invite, Refusal::Unknown)
    );
    assert!(matches!(
        p.invite(INVITE, Role::Owner, ISSUER, Role::Member, None, [3; 32]),
        Event::Invited { .. }
    ));
}

#[test]
fn recovery_needs_an_unlocked_panel_and_takes_one_offer_and_its_code() {
    let mut p = Pairing::new(Config::DEFAULT);
    assert_eq!(p.recovery(0), refused(Path::Recovery, Refusal::Closed));
    assert_eq!(
        p.gesture(0, recovery_hold(), Lock::Locked),
        Some(refused(Path::Recovery, Refusal::Locked))
    );
    assert!(!p.recovery_open());

    assert_eq!(
        p.gesture(0, recovery_hold(), Lock::Unlocked),
        Some(Event::RecoveryOpened)
    );
    assert_eq!(
        p.recovery(RECOVERY - 1),
        Event::Confirming {
            path: Path::Recovery
        }
    );
    assert_eq!(
        p.recovery(RECOVERY - 1),
        refused(Path::Recovery, Refusal::Closed),
        "the offer closed the window"
    );
    assert_eq!(p.tick(RECOVERY), None, "already closed by the offer");

    // The owner keys the client's code in, even on a locked panel.
    let code = code();
    let (last, first) = code.presses().split_last().unwrap();
    for &button in first {
        assert!(matches!(
            tap(&mut p, RECOVERY, button, Lock::Locked),
            Some(Event::Keyed {
                path: Path::Recovery,
                ..
            })
        ));
    }
    assert_eq!(
        tap(&mut p, RECOVERY, *last, Lock::Locked),
        Some(Event::Entered {
            path: Path::Recovery,
            code
        })
    );
}

/// The owner's retried recovery hold, with an attacker's offer pending,
/// keys in nothing: Lock going down before the chord forms is no press,
/// the chord refuses the offer, and the hold opens the window again for
/// the owner's own client. Six retries never enter an all-Lock code.
#[test]
fn a_retried_recovery_hold_refuses_a_pending_code_rather_than_key_it_in() {
    let mut p = Pairing::new(Config::DEFAULT);
    let mut now = 0;
    for gesture in chord_hold(Button::Dispense) {
        p.gesture(now, gesture, Lock::Unlocked);
    }
    assert_eq!(
        p.recovery(now + 50),
        Event::Confirming {
            path: Path::Recovery
        }
    );

    let mut events = Vec::new();
    for _ in 0..6 {
        now += 8_000;
        for gesture in chord_hold(Button::Dispense) {
            events.extend(p.gesture(now, gesture, Lock::Unlocked));
        }
        if p.awaiting().is_none() && p.recovery_open() {
            // The attacker offers again as soon as the window reopens.
            p.recovery(now + 50);
        }
    }

    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Keyed { .. } | Event::Entered { .. })),
        "{events:?}"
    );
    let refusals = events
        .iter()
        .filter(|&&e| e == refused(Path::Recovery, Refusal::Unconfirmed))
        .count();
    assert_eq!(refusals, 6, "each hold refused the pending offer");
    assert_eq!(
        events
            .iter()
            .filter(|&&e| e == Event::RecoveryOpened)
            .count(),
        6
    );
}

/// The setup hold, a lone hold or a press that began before the offer key
/// in nothing either; a chord or a hold refuses the offer.
#[test]
fn a_chord_or_a_hold_refuses_a_pending_code_and_counts_nothing() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.setup(0);
    let events: Vec<_> = chord_hold(Button::Mic)
        .into_iter()
        .filter_map(|gesture| p.gesture(10, gesture, Lock::Unlocked))
        .collect();
    assert_eq!(
        events,
        [
            Event::Pressed { path: Path::Setup },
            refused(Path::Setup, Refusal::Unconfirmed),
            Event::SetupOpened,
        ],
        "the hold acts as with nothing pending"
    );

    p.setup(20);
    assert_eq!(
        p.gesture(30, press(Button::Reset), Lock::Unlocked),
        Some(Event::Pressed { path: Path::Setup })
    );
    let hold = Gesture::Hold {
        chord: Buttons::of(Button::Reset),
        level: 1,
    };
    assert_eq!(
        p.gesture(40, hold, Lock::Unlocked),
        Some(refused(Path::Setup, Refusal::Unconfirmed))
    );

    // A release whose press went down before the offer counts nothing.
    p.setup(50);
    let up = Gesture::Up {
        chord: Buttons::of(Button::Dispense),
        level: 0,
        held_ms: 100,
    };
    assert_eq!(
        p.gesture(60, up, Lock::Unlocked),
        Some(Event::Swallowed { path: Path::Setup })
    );
    assert_eq!(
        p.awaiting(),
        Some(Path::Setup),
        "still waiting for its code"
    );
}

/// Six presses of one button key in no code: someone pressing Dispense
/// over and over for a feed that does not come would key in a code of
/// Dispense alone, so the feeder refuses any code of one button.
#[test]
fn a_code_of_one_button_is_refused() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.setup(0);
    let mut last = None;
    for i in 0..6 {
        last = tap(&mut p, 1_000 * i, Button::Dispense, Lock::Unlocked);
    }
    assert_eq!(last, Some(refused(Path::Setup, Refusal::Unconfirmed)));
    assert_eq!(p.awaiting(), None);
}

#[test]
fn a_recovery_code_never_keyed_in_is_unconfirmed() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.gesture(0, recovery_hold(), Lock::Unlocked);
    p.recovery(10);
    assert_eq!(
        p.tick(10 + CONFIRM),
        Some(refused(Path::Recovery, Refusal::Unconfirmed))
    );
}

#[test]
fn an_unused_recovery_window_closes() {
    let mut p = Pairing::new(Config::DEFAULT);
    p.gesture(0, recovery_hold(), Lock::Unlocked);
    assert_eq!(p.tick(RECOVERY - 1), None);
    assert_eq!(p.tick(RECOVERY), Some(Event::RecoveryClosed));
    assert_eq!(
        p.recovery(RECOVERY + 1),
        refused(Path::Recovery, Refusal::Closed)
    );
}

#[test]
fn a_lone_lock_hold_is_the_panels() {
    let mut p = Pairing::new(Config::DEFAULT);
    let hold = Gesture::Hold {
        chord: Buttons::of(Button::Lock),
        level: 1,
    };
    assert_eq!(p.gesture(0, hold, Lock::Unlocked), None);
}

#[test]
fn lock_and_mic_held_open_setup_mode_only_when_unlocked() {
    let mut p = Pairing::new(Config::DEFAULT);
    let setup = Gesture::Hold {
        chord: Buttons::of(Button::Lock).with(Button::Mic),
        level: 1,
    };
    assert_eq!(
        p.gesture(0, setup, Lock::Locked),
        Some(refused(Path::Setup, Refusal::Locked))
    );
    assert_eq!(
        p.gesture(0, setup, Lock::Unlocked),
        Some(Event::SetupOpened)
    );

    // Growing the chord is not a hold: Lock then Mic pressed reach the panel.
    let grown = Gesture::Down {
        chord: Buttons::of(Button::Lock).with(Button::Mic),
    };
    assert_eq!(p.gesture(0, grown, Lock::Unlocked), None);
}

#[test]
fn a_cancelled_setup_leaves_the_next_press_to_the_panel() {
    let mut p = Pairing::new(Config::DEFAULT);
    assert_eq!(p.setup(0), Event::Confirming { path: Path::Setup });
    p.cancel(Path::Signer);
    p.cancel(Path::Setup);
    assert_eq!(p.gesture(10, press(Button::Dispense), Lock::Unlocked), None);
    assert_eq!(p.setup(20), Event::Confirming { path: Path::Setup });
    p.gesture(30, press(Button::Mic), Lock::Unlocked);
    p.cancel(Path::Setup);
    assert_eq!(p.gesture(40, press(Button::Dispense), Lock::Unlocked), None);
}
