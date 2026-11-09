//! The calibration, plans and meal call stores, each saved inside a marked
//! attempt; a meal call change waits for a call without holding up the rest.

use std::cell::Cell;

use wellfed_device_api::mailbox::{SettingsFields, SettingsResult};
use wellfed_device_runtime::writer::{self as writer, WriteEvent};
use wellfed_feeder_runtime::writer::{FeederRecord, Record, Writer};
use wellfed_hal::flash::peer::MARKER_REGISTER;
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::settings;

use crate::support::{ATTEMPT_SEEN, open_all, start, step};

#[test]
fn a_calibration_is_saved_inside_a_marked_attempt_and_published() {
    use wellfed_device_api::mailbox::{Calibration, CalibrationResult};
    use wellfed_feeder_runtime::calibration;

    let (_guard, mut writer, mut provider) = start();
    calibration::reset();
    assert_eq!(
        calibration::current(),
        CalibrationResult::Busy,
        "not opened yet"
    );
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    let CalibrationResult::Current { view, pending } = calibration::current() else {
        panic!("the store is published");
    };
    assert_eq!(
        (view.calibration, view.generation, pending, view.writable),
        (Calibration::default(), 0, false, true)
    );

    let measured = Calibration {
        portion_mg: Some(9_800),
    };
    assert!(matches!(
        calibration::change(measured),
        CalibrationResult::Current { pending: true, .. }
    ));
    assert_eq!(calibration::change(measured), CalibrationResult::Busy);
    ATTEMPT_SEEN.with(|s| s.set(false));
    let saved = Record::Product(FeederRecord::CalibrationSaved {
        generation: 1,
        portion_mg: Some(9_800),
    });
    assert_eq!(step(&mut writer, &mut provider), Some(saved));
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);

    // The same value again writes nothing.
    assert!(matches!(
        calibration::change(measured),
        CalibrationResult::Current { pending: true, .. }
    ));
    ATTEMPT_SEEN.with(|s| s.set(false));
    assert_eq!(step(&mut writer, &mut provider), Some(saved));
    assert!(!ATTEMPT_SEEN.with(Cell::get));

    // The next boot reads it back.
    calibration::reset();
    let mut writer = Writer::default();
    step(&mut writer, &mut provider);
    let CalibrationResult::Current { view, .. } = calibration::current() else {
        panic!("the store is published");
    };
    assert_eq!((view.calibration, view.generation), (measured, 1));

    let mut text = String::new();
    writer::describe(saved, &mut text).unwrap();
    assert_eq!(text, "calibration saved generation=1 portion_mg=9800");
}

#[test]
fn a_plans_change_is_applied_saved_inside_a_marked_attempt_and_published() {
    use wellfed_device_api::mailbox::{Claim, PLANS, Plan, PlansChange, PlansResult, Weekdays};
    use wellfed_feeder_runtime::plans;

    let (_guard, mut writer, mut provider) = start();
    plans::reset();
    assert_eq!(plans::current(), PlansResult::Busy, "not opened yet");
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save
    let PlansResult::Current { view, pending } = plans::current() else {
        panic!("the store is published");
    };
    assert_eq!(
        (view.table, view.revision, pending, view.writable),
        ([None; PLANS], 0, false, true)
    );

    let mut table = [None; PLANS];
    table[0] = Some(Plan {
        minute: 7 * 60 + 30,
        portions: 2,
        days: Weekdays::EVERY_DAY,
        enabled: true,
        call: 0,
    });
    let replace = PlansChange::Replace {
        table,
        at_s: Some(1_000),
    };
    assert!(matches!(
        plans::change(replace, None),
        PlansResult::Current { pending: true, .. }
    ));
    assert_eq!(plans::change(replace, None), PlansResult::Busy);
    ATTEMPT_SEEN.with(|s| s.set(false));
    let saved = Record::Product(FeederRecord::PlansSaved {
        sequence: 1,
        revision: 1,
        event: WriteEvent::None,
    });
    assert_eq!(step(&mut writer, &mut provider), Some(saved));
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(sdk::backup_read(MARKER_REGISTER), 0);

    // A claim against a revision the table has left is dropped unwritten.
    let claim = Claim {
        plan: 0,
        due_s: 2_000,
        high_water: 7,
        disabled: false,
    };
    let stale = PlansChange::Claim { claim, revision: 0 };
    assert!(matches!(
        plans::change(stale, None),
        PlansResult::Current { pending: true, .. }
    ));
    ATTEMPT_SEEN.with(|s| s.set(false));
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::PlansDropped))
    );
    assert!(!ATTEMPT_SEEN.with(Cell::get));

    let current = PlansChange::Claim { claim, revision: 1 };
    assert!(matches!(
        plans::change(current, None),
        PlansResult::Current { pending: true, .. }
    ));
    step(&mut writer, &mut provider);

    // The next boot reads both back.
    plans::reset();
    let mut writer = Writer::default();
    step(&mut writer, &mut provider);
    let PlansResult::Current { view, .. } = plans::current() else {
        panic!("the store is published");
    };
    assert_eq!((view.table, view.claim), (table, Some(claim)));

    let mut text = String::new();
    writer::describe(saved, &mut text).unwrap();
    assert_eq!(text, "plans saved sequence=1 revision=1 event=none");
}

/// The meal call's upload lands chunk by chunk inside marked attempts. A
/// change queued while a call plays from the clip waits without holding
/// up the writer's other work, and one asked for meanwhile is refused.
#[test]
fn a_meal_call_change_waits_for_a_call_without_holding_up_other_work() {
    use wellfed_core::store::slot::crc32;
    use wellfed_device_api::mailbox::{CallChange, CallClip, CallResult};
    use wellfed_feeder_runtime::call;
    use wellfed_hal::audio::{self, PlayResult, Record as Audio, Speaker};

    let (_guard, mut writer, mut provider) = start();
    call::reset();
    audio::reset();
    open_all(&mut writer, &mut provider);
    step(&mut writer, &mut provider); // the stats store's early save

    let stream: Vec<u8> = (0..600u32).map(|i| (i * 7) as u8).collect();
    let crc = crc32(&stream);
    let start = CallChange::Start { bytes: 600, crc };
    assert!(matches!(
        call::change(start),
        CallResult::Current { pending: true, .. }
    ));
    ATTEMPT_SEEN.with(|s| s.set(false));
    let receiving = CallClip::Receiving {
        bytes: 600,
        crc,
        received: 0,
    };
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::Call { clip: receiving }))
    );
    assert!(ATTEMPT_SEEN.with(Cell::get));
    assert_eq!(call::chunk(0, &stream[..512]), Ok(()));
    assert_eq!(step(&mut writer, &mut provider), None, "mid-upload");
    assert_eq!(call::chunk(512, &stream[512..]), Ok(()));
    let stored = CallClip::Stored { bytes: 600, crc };
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::Call { clip: stored }))
    );

    // A delete queued, then a call takes the clip before the writer's turn.
    assert!(matches!(
        call::change(CallChange::Delete),
        CallResult::Current { pending: true, .. }
    ));
    assert_eq!(audio::request_call(1), PlayResult::Started);
    let mut speaker = Speaker::quiet();
    assert!(matches!(
        speaker.step().record,
        Some(Audio::CallPlay { plays: 1, .. })
    ));
    assert!(audio::call_playing());

    // The settings change behind it is saved meanwhile; the delete waits.
    let quieter = SettingsFields {
        volume: Some(10),
        ..SettingsFields::default()
    };
    assert!(matches!(
        settings::request(quieter),
        SettingsResult::Current { pending: true, .. }
    ));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Saved { .. })
    ));
    let CallResult::Current { view, pending } = call::current() else {
        panic!("the call is published");
    };
    assert_eq!((view.clip, pending), (stored, true));

    // Once the call ends, the delete lands.
    for _ in 0..400 {
        let step = speaker.step();
        fake::with(|h| h.uptime_ms += step.delay_ms);
        fake::play_audio(2);
        if matches!(step.record, Some(Audio::CallDone { .. })) {
            break;
        }
    }
    assert!(!audio::call_playing());
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::Product(FeederRecord::Call {
            clip: CallClip::None
        }))
    );
    // Refused, not queued, while a call plays.
    assert!(matches!(
        call::change(start),
        CallResult::Current { pending: true, .. }
    ));
    step(&mut writer, &mut provider);
    assert_eq!(call::chunk(0, &stream[..512]), Ok(()));
    step(&mut writer, &mut provider);
    assert_eq!(call::chunk(512, &stream[512..]), Ok(()));
    step(&mut writer, &mut provider);
    assert_eq!(audio::request_call(1), PlayResult::Started);
    let mut speaker = Speaker::quiet();
    speaker.step();
    assert_eq!(call::change(CallChange::Delete), CallResult::Busy);
}
