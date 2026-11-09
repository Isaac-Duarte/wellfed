//! A client of an older build reads what a newer server sends. Each enum
//! the API may grow has a catch-all, which a value this build does not
//! know decodes as, so a body with one still reads; the `api_growth` test
//! in `wellfed-server` refuses growth of an enum without one.

use serde::de::DeserializeOwned;
use wellfed_api::{
    Blocked, Bowl, ClientRole, CommandKind, CommandOutcome, CommandState, Connection, CurrentUnit,
    Desiccant, DeviceKind, DeviceStatus, ErrorCode, FeedOutcome, FeedReason, Feeder, FeederRefusal,
    FoodLevel, InstallState, MaybeKnown, MealReminder, Model, Phase, Problem, Role, Settings,
    Source, Units, WifiSource, Window,
};
use wellfed_model::{Bank0Blocked, Bank0Check, Skew};

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

/// A device of a kind or model a newer server added still lists, as
/// unknown, so a client shows it as needing an update and reads the rest
/// of `GET /feeders`.
#[test]
fn a_device_a_newer_build_added_lists_as_unknown() {
    let entry = |kind: &str, model: &str| {
        format!(
            r#"{{"id":"7f3d1c2e-0000-4000-8000-000000000001","home":"7f3d1c2e-0000-4000-8000-000000000002",
            "kind":"{kind}","model":"{model}","name":"x","identity":"{identity}","role":"owner",
            "address":null,"connection":"online","skew":null,"synced":null,"epoch":null,"archived":null}}"#,
            identity = "ab".repeat(32),
        )
    };
    let list = format!(
        "[{},{},{}]",
        entry("feeder", Model::ALL[0].name()),
        entry("a_newer_kind", "a_newer_model"),
        entry("feeder", "a_newer_model"),
    );

    let feeders = read::<Vec<Feeder>>(&list);
    assert_eq!(
        feeders
            .iter()
            .map(|feeder| (feeder.kind, feeder.model))
            .collect::<Vec<_>>(),
        [
            (
                MaybeKnown::Known(DeviceKind::Feeder),
                MaybeKnown::Known(Model::ALL[0])
            ),
            (MaybeKnown::Unknown, MaybeKnown::Unknown),
            (MaybeKnown::Known(DeviceKind::Feeder), MaybeKnown::Unknown),
        ]
    );

    let status = read::<DeviceStatus>(r#"{"kind":"a_newer_kind","pump":"on"}"#);
    assert_eq!(status, DeviceStatus::Unknown);
    assert_eq!(status.kind(), MaybeKnown::Unknown);
    let settings = read::<Settings>(r#"{"kind":"a_newer_kind","generation":3,"flow":2}"#);
    assert_eq!(settings, Settings::Unknown);
    assert_eq!(settings.generation(), None);
}

/// Bank 0's check is adjacently tagged, where serde's own catch-all fails
/// on a newer state that carries a reason: any newer state reads as
/// unknown, and the known ones read as written.
#[test]
fn a_bank0_check_a_newer_build_added_reads_as_unknown() {
    for newer in [
        r#"{"state":"x"}"#,
        r#"{"state":"x","reason":"y"}"#,
        r#"{"state":"x","reason":{"z":1}}"#,
    ] {
        assert_eq!(read::<Bank0Check>(newer), Bank0Check::Unknown, "{newer}");
    }

    for known in [
        Bank0Check::Unchecked,
        Bank0Check::Match,
        Bank0Check::Blocked(Bank0Blocked::Marker),
    ] {
        let json = serde_json::to_string(&known).unwrap();
        assert_eq!(read::<Bank0Check>(&json), known, "{json}");
    }
    assert_eq!(
        read::<Bank0Check>(r#"{"state":"blocked","reason":"a_newer_reason"}"#),
        Bank0Check::Blocked(Bank0Blocked::Unknown)
    );
    assert!(serde_json::from_str::<Bank0Check>(r#"{"state":"blocked"}"#).is_err());
}
