//! Rules that hold on every path of the Wi-Fi task, whatever the driver
//! answers and whenever a restart or battery mode asks. The scripted tests
//! follow chosen paths; these drive arbitrary ones.
//!
//! - A `Select` takes the join's turn from feeds, and exactly one `Outcome`
//!   gives it back before the next `Select` or radio change.
//! - A `TurnBegin` is closed by `TurnEnd`, or by `RadioFailed`, which
//!   restarts with it held.
//! - The task never parks or ends holding either turn: every feed after
//!   that would be refused.
//! - Once a restart is asked for, nothing scans or connects again, and
//!   nothing does while battery mode wants the radio off.
//! - Every step returns (a cycle without an action would hang the test).

use proptest::prelude::*;
use wellfed_core::echo::Outcome as Echo;
use wellfed_core::liveness::Radio;
use wellfed_core::wifi_task::{Action, Done, Sighting, WifiTask, Wishes};

const GATEWAY: u32 = 0xc0a8_0101;
const IP: u32 = 0xc0a8_0123;
const SECURITY: u32 = 0x0040_0004;

/// One step of the schedule: what the driver answers, how much time passes,
/// and what the rest of the firmware wants.
#[derive(Clone, Copy, Debug)]
struct Turn {
    value: u32,
    /// The driver answers with a variant the action does not name, which
    /// only a driver bug would.
    wrong: bool,
    elapsed: u32,
    restart: bool,
    radio_off: bool,
}

fn turn() -> impl Strategy<Value = Turn> {
    (
        any::<u32>(),
        prop::bool::weighted(0.03),
        prop_oneof![Just(0u32), 0..120_000u32, any::<u32>()],
        prop::bool::weighted(0.005),
        prop::bool::weighted(0.15),
    )
        .prop_map(|(value, wrong, elapsed, restart, radio_off)| Turn {
            value,
            wrong,
            elapsed,
            restart,
            radio_off,
        })
}

/// What a driver might answer `action`, from `value`'s bits.
fn answer(action: Action, value: u32, wrong: bool) -> Done {
    let bit = |n: u32| value >> n & 1 == 1;
    if wrong {
        return if matches!(action, Action::Gateway) {
            Done::Nothing
        } else {
            Done::Gateway(value)
        };
    }
    match action {
        Action::RadioOn
        | Action::RadioOff
        | Action::Disconnect
        | Action::Scan
        | Action::Connect { .. } => Done::Status(if value % 4 == 0 { -1 } else { 0 }),
        Action::Associated => Done::Associated(bit(0)),
        Action::Select => Done::Selected((value % 5 != 0).then_some(value % 3)),
        Action::Sighting => Done::Sighting(Sighting {
            done: bit(0),
            found: bit(1),
            rssi: -60,
            security: SECURITY,
        }),
        Action::DhcpStart => {
            let state = u8::try_from(value % 4).unwrap();
            Done::Dhcp {
                state,
                ip: if state == 2 { IP } else { 0 },
            }
        }
        Action::AccessPoint => Done::AccessPoint {
            channel: bit(0).then_some(6),
            bssid: bit(1).then_some([2, 0x11, 0x22, 0x33, 0x44, 0x55]),
        },
        Action::Lease => Done::Lease {
            used: value % 4000,
            bound: bit(2),
        },
        Action::Link => Done::Link {
            associated: bit(0),
            leased: bit(1),
        },
        Action::Gateway => Done::Gateway(if bit(0) { GATEWAY } else { 0 }),
        Action::Echo { .. } => Done::Echo(
            [
                Echo::Answered,
                Echo::NoGateway,
                Echo::NoSocket,
                Echo::NotSent,
                Echo::Silent,
            ][usize::try_from(value % 5).unwrap()],
        ),
        _ => Done::Nothing,
    }
}

/// Drive the task through `turns` and check every rule over its actions.
fn run(start: u32, turns: &[Turn]) -> Result<(), TestCaseError> {
    let mut task = WifiTask::default();
    let mut now = start;
    let mut done = Done::Nothing;
    let mut wishes = Wishes::NONE;
    let (mut joining, mut turn_held) = (false, false);

    for (index, turn) in turns.iter().enumerate() {
        // A restart, once asked for, stays asked for.
        wishes.restarting |= turn.restart;
        wishes.radio = if turn.radio_off {
            Radio::Off
        } else {
            Radio::On
        };
        now = now.wrapping_add(turn.elapsed);

        let action = task.step(now, wishes, done);
        let at = format!("step {index}: {action:?} under {wishes:?}");
        match action {
            Action::Select => {
                prop_assert!(!joining && !turn_held, "{at}");
                joining = true;
            }
            Action::Outcome(_) => {
                prop_assert!(joining, "an outcome with no join: {at}");
                joining = false;
            }
            Action::TurnBegin => {
                prop_assert!(!joining && !turn_held, "{at}");
                turn_held = true;
            }
            Action::TurnEnd => {
                prop_assert!(turn_held, "a turn ended that never began: {at}");
                turn_held = false;
            }
            Action::RadioFailed => return Ok(()),
            Action::Scan | Action::Connect { .. } => {
                prop_assert!(!wishes.restarting, "a restart is coming: {at}");
                prop_assert!(wishes.radio == Radio::On, "the radio is wanted off: {at}");
            }
            Action::Park | Action::Stop => {
                prop_assert!(!joining && !turn_held, "parked holding a turn: {at}");
                return Ok(());
            }
            Action::Delay(ms) => now = now.wrapping_add(ms),
            _ => {}
        }
        done = answer(action, turn.value, turn.wrong);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn every_turn_is_given_back_and_a_restart_stops_every_join(
        start in any::<u32>(),
        turns in proptest::collection::vec(turn(), 1..600),
    ) {
        run(start, &turns)?;
    }
}
