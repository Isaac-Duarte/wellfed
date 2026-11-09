//! The settings writer over a product with no stores of its own, against
//! the fake SDK storage: the device's stores open, publish and save without
//! any kind's policy linked, as a device kind other than the feeder would
//! build on them.

use std::cell::Cell;
use std::fmt;

use wellfed_core::settings::{SettingsFields, SettingsResult};
use wellfed_core::store::update::UpdateStatus;
use wellfed_device::store::events::EventsFill;
use wellfed_device::store::reset;
use wellfed_device::store::settings::{KindStore, SettingsStatus, Stored};
use wellfed_device_runtime::writer::{
    self as writer, Core, Describe, POLL_MS, Product, RESET_BASE, Record, Step, Wear, Writer,
};
use wellfed_device_runtime::{events, keys, network};
use wellfed_hal::board::storage::{FACTORY_CLEARED_SECTORS, UPDATE};
use wellfed_hal::flash::park::Mmio;
use wellfed_hal::flash_lock::arming::Hooks;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::ota::update::{self as update, SdkFlash, StoreSlots, Update};
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::settings::Mailbox;
use wellfed_hal::supervision;
use wellfed_sdk::storage::{FlashOps, Region};

thread_local! {
    /// What the product saw of the writer's order.
    static FILLS: Cell<u32> = const { Cell::new(0) };
    static PUBLISHED: Cell<u32> = const { Cell::new(0) };
}

/// A product's record, which this one never makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Never {}

impl Describe for Never {
    fn describe(self, _: &mut impl fmt::Write) -> fmt::Result {
        match self {}
    }
}

/// The settings mailbox of a kind with no settings of its own.
static SETTINGS: Mailbox<()> = Mailbox::new();

/// A device kind with no stores beyond the device's.
#[derive(Default)]
struct Bare;

impl<F: FlashOps> Product<F> for Bare {
    type Record = Never;
    type Settings = ();

    fn settings_mailbox() -> &'static Mailbox<()> {
        &SETTINGS
    }

    fn settings(&self) -> Stored<()> {
        ().stored()
    }

    fn save_settings(&mut self, (): &(), _: u64) -> bool {
        true
    }

    fn reopen_settings(&mut self) {}

    fn settings_record(&self) -> Option<Never> {
        None
    }

    fn open_kept(&mut self) {}

    fn open_cleared(&mut self) {}

    fn reopen_handed_over(&mut self) {}

    fn publish(&self, _: bool) {
        PUBLISHED.with(|p| p.set(p.get() + 1));
    }

    fn open_with_events(&mut self) {}

    fn open_late(&mut self) -> Option<Never> {
        None
    }

    fn events_filled(_: EventsFill) {
        FILLS.with(|f| f.set(f.get() + 1));
    }

    fn mailboxes<H: Hooks>(&mut self, _: &mut Core<F>) -> Option<Step<Never>> {
        None
    }

    fn rest(&mut self, _: &mut Core<F>, _: bool) -> Option<Step<Never>> {
        None
    }

    fn keep_on_reset() -> Option<(u32, Wear)> {
        None
    }
}

struct NoMmio;

impl Mmio for NoMmio {
    const KM0: bool = false;
    fn load(_: u32) -> u32 {
        unreachable!("no arming with the hooks armed")
    }
    fn store(_: u32, _: u32) {
        unreachable!("no arming with the hooks armed")
    }
    fn barrier() {}
    fn mask() -> u32 {
        0
    }
    fn halt() -> ! {
        panic!("halt")
    }
}

struct Armed;

impl Hooks for Armed {
    fn watchdog() -> bool {
        true
    }
    fn install(_: *mut Provider) -> bool {
        true
    }
    fn armed() -> bool {
        true
    }
}

impl FlashOps for Armed {
    fn install(_: *mut Provider) -> bool {
        true
    }
    fn read_id() -> [u8; 3] {
        [0; 3]
    }
    fn armed() -> bool {
        true
    }
    fn erase(sector: u32) -> bool {
        // SAFETY: the fake's storage, which this test owns.
        unsafe { sdk::storage_erase(sector) }
    }
    fn program(sector: u32, offset: u32, data: &[u8]) -> bool {
        // SAFETY: as in `erase`.
        unsafe { sdk::storage_program(sector, offset, data) }
    }
    fn read(sector: u32, offset: u32, bytes: &mut [u8]) -> bool {
        sdk::storage_read(sector, offset, bytes)
    }
}

fn step(writer: &mut Writer<Armed, Bare>, provider: &mut Provider) -> Option<Record<Never>> {
    // SAFETY: the provider outlives the run.
    unsafe { writer.step::<NoMmio, Armed>(provider) }.record
}

/// The fake hardware and the runtime's mailboxes at boot, and a writer.
fn boot() -> (Writer<Armed, Bare>, Provider) {
    fake::reset_hardware();
    SETTINGS.reset();
    // SAFETY: this test runs alone in its process and no Wi-Fi task runs.
    unsafe { network::reset() };
    keys::reset();
    events::reset();
    INTERLOCK.reset();
    supervision::reset();
    wellfed_hal::writer_wake::reset();
    (Writer::default(), Provider::default())
}

#[test]
fn a_product_without_stores_keeps_the_devices() {
    let (mut writer, mut provider) = boot();

    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Open {
            status: SettingsStatus::Empty,
            ..
        })
    ));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::NetOpen { .. })
    ));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::TimeOpen { .. })
    ));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Keys { .. })
    ));
    assert!(matches!(
        step(&mut writer, &mut provider),
        Some(Record::Events { open: true, .. })
    ));
    assert_eq!(
        FILLS.with(Cell::get),
        1,
        "the ring's fill goes to the product"
    );
    assert!(
        PUBLISHED.with(Cell::get) > 0,
        "the product publishes with the device"
    );

    let fields = SettingsFields {
        volume: Some(20),
        ..SettingsFields::default()
    };
    assert!(matches!(
        SETTINGS.request(fields),
        SettingsResult::Current { pending: true, .. }
    ));
    let mut text = String::new();
    let saved = step(&mut writer, &mut provider).expect("the change is saved");
    writer::describe(saved, &mut text).unwrap();
    assert!(text.starts_with("saved generation=1 "), "{text}");

    // SAFETY: as above.
    let idle = unsafe { writer.step::<NoMmio, Armed>(&mut provider) };
    assert_eq!((idle.delay_ms, idle.record), (POLL_MS, None));
}

#[test]
fn a_factory_reset_with_nothing_kept_erases_the_marker_after_the_cleared_sectors_and_the_update_step()
 {
    let (mut writer, mut provider) = boot();
    assert!(reset::mark(&Region::<Armed, RESET_BASE, 1>::new()));

    assert_eq!(step(&mut writer, &mut provider), Some(Record::ResetPending));
    for index in 0..FACTORY_CLEARED_SECTORS {
        assert_eq!(
            step(&mut writer, &mut provider),
            None,
            "cleared sector {index} erases as its own step"
        );
        assert!(reset::pending(&Region::<Armed, RESET_BASE, 1>::new()));
    }
    assert_eq!(
        step(&mut writer, &mut provider),
        None,
        "the update store's step, with nothing to clear"
    );
    assert_eq!(
        step(&mut writer, &mut provider),
        Some(Record::ResetDone {
            erased: FACTORY_CLEARED_SECTORS
        }),
        "with no keep step the marker goes next"
    );
    assert!(!reset::pending(&Region::<Armed, RESET_BASE, 1>::new()));
}

/// The update stage runs on another task and may have opened a corrupt
/// store before the reset clears it: the writer tells it to read the store
/// again, and it serves the empty store without waiting for a restart.
#[test]
fn a_reset_by_hand_over_a_corrupt_update_store_reopens_the_update_stage_empty() {
    let (mut writer, mut provider) = boot();
    let mut junk = [0u8; 384];
    junk[..8].copy_from_slice(b"PLFUPD01");
    junk[8] = 2;
    junk[10..12].copy_from_slice(&384u16.to_le_bytes());
    junk[380..].copy_from_slice(b"CMIT");
    // SAFETY: the fake's storage, which this test owns.
    assert!(unsafe { sdk::storage_program(UPDATE.first, 0, &junk) });

    let mut stage = Box::new(Update::new(1, [0xee; 32], None, StoreSlots::<Armed>::new()));
    let mut flash = SdkFlash::<Armed>::new();
    let opened = |step: update::Step| match step.record {
        Some(update::Record::Opened { status, .. }) => Some(status),
        _ => None,
    };
    assert_eq!(
        opened(stage.step(&mut flash, 1)),
        Some(UpdateStatus::Corrupt)
    );

    assert!(reset::mark(&Region::<Armed, RESET_BASE, 1>::new()));
    let records: Vec<Record<Never>> = (0..FACTORY_CLEARED_SECTORS + 3)
        .filter_map(|_| step(&mut writer, &mut provider))
        .collect();
    assert!(!reset::pending(&Region::<Armed, RESET_BASE, 1>::new()));
    assert!(records.contains(&Record::ResetUpdateCleared), "{records:?}");

    assert_eq!(opened(stage.step(&mut flash, 2)), Some(UpdateStatus::Empty));
}
