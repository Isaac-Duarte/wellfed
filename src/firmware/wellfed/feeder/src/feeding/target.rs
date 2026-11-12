//! The adapter's runtime on the board: the owner in its static, the
//! runtime's flags, the board's interlock, the trace store and the log.

use wellfed_core_feeder::feed::observe::Observation;
use wellfed_core_feeder::feed::progress::Direction;
use wellfed_core_feeder::feed_journal::{FeedJournal, JournalError};
use wellfed_feeder::trace::DriveFigures;
use wellfed_feeder_runtime::traces;
use wellfed_hal_feeder::owner::{Owner, OwnerConfig};
use wellfed_proto::api::{EventBody, TraceSummary};
use wellfed_sdk::hal::interlock::{INTERLOCK, Interlock};
use wellfed_sdk::hal::supervision::supervise;
use wellfed_sdk::{JournalSlots, Slot, SlotGuard};

use super::adapter::{self, Opening, Runtime};
use super::{CONFIG, JOURNAL, PowerCycle, Record};

type Journal = FeedJournal<JournalSlots>;

/// The owner. Reached only through `&`: the supervision interrupts reach
/// its board and supervisor while the loop runs, so no `&mut` may cover
/// it once it is open. Filled once, before it opens.
static OWNER: Slot<Owner<Journal>> = Slot::new();

/// The loop's half of the feeding owner on this board.
pub type Feeding = adapter::Feeding<Target>;

/// End a food reading now: Bluetooth setup is starting in this pass, ahead
/// of the next step's hold. For the platform, which holds no `Feeding`; the
/// owner is reached through its static, as in a pass.
pub fn yield_food_read() {
    if let Some(owner) = OWNER.enter() {
        owner.yield_food_read(&INTERLOCK);
    }
}

/// The runtime as the KM4 image has it.
#[derive(Debug, Default)]
pub struct Target;

impl Runtime for Target {
    type Owner = Owner<Journal>;
    type Entered<'a> = SlotGuard<'a, Owner<Journal>>;

    fn owner(&self) -> Option<Self::Entered<'_>> {
        OWNER.enter()
    }

    /// Opens the journal (creating it on erased sectors, or over a first
    /// creation a power cut interrupted), then the owner on it.
    fn open(&self, created: &mut bool) -> Option<Opening> {
        // The `owner-fail` variant: a build whose owner cannot open, so
        // its trial must not confirm. The journal is left untouched.
        if cfg!(feature = "owner-fail") {
            return Some(Opening {
                record: Record::Journal {
                    error: JournalError::Corrupt,
                    created: false,
                },
                interrupted: None,
            });
        }
        let io = JournalSlots::new();
        let mut journal = Journal::default();
        let mut error = journal.open(io, JOURNAL);
        if error == JournalError::Empty {
            // First boot of the feeding owner: both sectors erased, or
            // the first boot's creation cut short.
            *created = true;
            error = journal.create(io, JOURNAL, Direction::D0, true, true);
        }
        let created = *created;
        if error != JournalError::Ok {
            return Some(Opening {
                record: Record::Journal { error, created },
                interrupted: None,
            });
        }
        let high_water = journal.view.recovery.high_water_id;
        let interrupted = journal.view.recovery.pending_request.then_some(high_water);

        // Filled here, before the owner opens, so no interrupt reaches it yet.
        let owner = OWNER.enter_or_init(Owner::default)?;
        let config = OwnerConfig {
            flash: wellfed_sdk::flash_authority(),
            ..CONFIG
        };
        // SAFETY: the authority is the runtime's static, live for the
        // boot; the owner is a static and never moves.
        let opened = unsafe { owner.open(&config, journal) };
        Some(if opened {
            Opening {
                record: Record::Opened {
                    created,
                    high_water,
                },
                interrupted,
            }
        } else {
            Opening {
                record: Record::Refused,
                interrupted: None,
            }
        })
    }

    fn interlock(&self) -> &Interlock {
        &INTERLOCK
    }

    fn now_ms(&self) -> u32 {
        wellfed_sdk::hal::sdk::uptime_ms()
    }

    fn held(&self) -> bool {
        wellfed_sdk::held()
    }

    fn confirming(&self) -> bool {
        wellfed_sdk::confirming()
    }

    fn storage_armed(&self) -> bool {
        wellfed_sdk::storage_armed()
    }

    fn installing(&self) -> bool {
        wellfed_sdk::hal::ota::mailbox::installing()
    }

    fn writes_queued(&self) -> bool {
        super::writes_queued()
    }

    /// The KM4 boot count the flash arming keeps in a backup register.
    fn power_cycle(&self) -> PowerCycle {
        use wellfed_sdk::hal::flash_lock::provider::BOOT_COUNT_REGISTER;
        if wellfed_sdk::hal::sdk::backup_read(BOOT_COUNT_REGISTER) <= 1 {
            PowerCycle::FirstBoot
        } else {
            PowerCycle::Restarted
        }
    }

    fn report_feeding(&self, feeding: supervise::Feeding) {
        wellfed_sdk::report_feeding(feeding);
    }

    fn request_reset(&self) {
        wellfed_sdk::request_reset();
    }

    fn record(&self, record: Record) {
        wellfed_sdk::log(format_args!("PLOWN1 {record}\r\n"));
    }

    fn trace_begin(&self, feed: u32) {
        traces::begin(feed);
    }

    fn trace_observe(&self, seen: &Observation) {
        traces::observe(seen);
    }

    fn trace_seal(&self, fed: &EventBody, drive: &DriveFigures) -> Option<TraceSummary> {
        traces::seal(fed, drive)
    }
}
