//! A client of an older build reads what a newer server sends. Each enum
//! the API may grow has a catch-all, which a value this build does not
//! know decodes as, so a body with one still reads; the `api_growth` test
//! in `wellfed-server` refuses growth of an enum without one.

use serde::de::DeserializeOwned;
use wellfed_api::{
    Blocked, Bowl, ClientRole, CommandKind, CommandOutcome, CommandState, Connection, CurrentUnit,
    Desiccant, ErrorCode, FeedOutcome, FeedReason, FeederRefusal, FoodLevel, InstallState,
    MealReminder, Phase, Problem, Role, Source, Units, WifiSource, Window,
};
use wellfed_model::Skew;

fn read<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|error| panic!("{json} does not read: {error}"))
}

#[test]
fn a_value_a_newer_server_added_reads_as_the_catch_all() {
    let newer = r#""a_newer_value""#;

    assert_eq!(read::<ErrorCode>(newer), ErrorCode::Unknown);
    assert_eq!(read::<Connection>(newer), Connection::Unknown);
    assert_eq!(read::<FeederRefusal>(newer), FeederRefusal::Unknown);
    assert_eq!(read::<FoodLevel>(newer), FoodLevel::Unknown);
    assert_eq!(read::<FeedOutcome>(newer), FeedOutcome::Unknown);
    assert_eq!(read::<FeedReason>(newer), FeedReason::Unknown);
    assert_eq!(read::<Blocked>(newer), Blocked::Unknown);
    assert_eq!(read::<CurrentUnit>(newer), CurrentUnit::Unknown);
    assert_eq!(read::<WifiSource>(newer), WifiSource::Unknown);
    assert_eq!(read::<Bowl>(newer), Bowl::Unknown);
    assert_eq!(read::<Role>(newer), Role::Unknown);
    assert_eq!(read::<ClientRole>(newer), ClientRole::Unknown);
    assert_eq!(read::<CommandKind>(newer), CommandKind::Unknown);
    assert_eq!(read::<InstallState>(newer), InstallState::Unknown);
    assert_eq!(read::<Skew>(newer), Skew::Unknown);
    assert_eq!(read::<Units>(newer), Units::Unknown);
    // A state a newer server added is uncertain: the write may have run.
    assert_eq!(read::<CommandState>(newer), CommandState::Unknown);
    // `unknown` is a resting place of its own; a newer one is `newer`.
    assert_eq!(read::<Phase>(newer), Phase::Newer);
    assert_eq!(read::<Phase>(r#""unknown""#), Phase::Unknown);
}

#[test]
fn a_tagged_value_a_newer_build_added_reads_as_the_catch_all() {
    assert_eq!(
        read::<Source>(r#"{"kind":"a_newer_source","x":1}"#),
        Source::Unknown
    );
    assert_eq!(
        read::<Window>(r#"{"mode":"a_newer_mode"}"#),
        Window::Unknown
    );
    assert_eq!(
        read::<MealReminder>(r#"{"mode":"a_newer_mode"}"#),
        MealReminder::Unknown
    );
    assert_eq!(
        read::<Desiccant>(r#"{"state":"a_newer_state"}"#),
        Desiccant::Unknown
    );

    // The known ones read as they always have.
    assert_eq!(read::<Window>(r#"{"mode":"always"}"#), Window::Always);
    assert_eq!(read::<Desiccant>(r#"{"state":"unset"}"#), Desiccant::Unset);
}

#[test]
fn an_outcome_a_newer_server_added_reads_as_unknown() {
    let outcome: CommandOutcome = read(r#"{"result":"a_newer_outcome","feed":3}"#);
    assert_eq!(outcome, CommandOutcome::Unknown);
    assert_eq!(outcome.feed(), None);

    // The known ones read as they always have.
    let feeding: CommandOutcome = read(r#"{"result":"feeding","feed":3}"#);
    assert_eq!(feeding, CommandOutcome::Feeding { feed: 3 });
}

#[test]
fn a_refusal_with_a_newer_code_is_still_a_refusal() {
    let problem: Problem = read(
        r#"{"type":"about:blank","title":"Something new","status":409,"code":"a_newer_code"}"#,
    );
    assert_eq!(problem.code, ErrorCode::Unknown);
    assert_eq!(problem.status, 409);
}
