//! The settings writer over a product with no stores of its own, against
//! the fake SDK storage: the device's stores open, publish and save without
//! any kind's policy linked, as a device kind other than the feeder would
//! build on them.

use std::cell::Cell;
use std::fmt;

use wellfed_device::store::events::EventsFill;
use wellfed_device::store::settings::SettingsStatus;
use wellfed_device_api::mailbox::{SettingsFields, SettingsResult};
use wellfed_device_runtime::writer::{
    self as writer, Core, Describe, POLL_MS, Product, Record, Step, Wear, Writer,
};
use wellfed_device_runtime::{events, keys, network};
use wellfed_hal::flash::park::Mmio;
use wellfed_hal::flash_lock::arming::Hooks;
use wellfed_hal::flash_lock::provider::Provider;
use wellfed_hal::interlock::INTERLOCK;
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::{settings, supervision};
use wellfed_sdk::storage::FlashOps;

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

/// A device kind with no stores beyond the device's.
#[derive(Default)]
struct Bare;

impl<F: FlashOps> Product<F> for Bare {
    type Record = Never;

    const HANDOVER_CLEARED: &'static [u32] = &[];

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

    fn keep_on_reset() -> (u32, Wear) {
        (writer::RESET_BASE, Wear::Kept)
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

#[test]
fn a_product_without_stores_keeps_the_devices() {
    fake::reset_hardware();
    settings::reset();
    // SAFETY: this test runs alone in its process and no Wi-Fi task runs.
    unsafe { network::reset() };
    keys::reset();
    events::reset();
    INTERLOCK.reset();
    supervision::reset();
    wellfed_hal::writer_wake::reset();
    let mut writer = Writer::<Armed, Bare>::default();
    let mut provider = Provider::default();

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
        settings::request(fields),
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
