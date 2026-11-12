//! The PLAF103 board, as the replacement firmware sees it: the pins and ADC
//! channels its ports use, and who owns each sector of the storage region.
//! These are facts of this feeder model, not of the AmebaD SoC, so the SoC
//! bindings (`wellfed-amebad-sys`) and every store read them from here;
//! a second model is a second crate of the same shape, chosen at build
//! time through `wellfed-board`, which checks that it holds the contract.
//! Everything is a compile-time constant: nothing reads the board at run
//! time.
#![no_std]

/// The model this board is, as `wellfed_proto::device::Model::code` numbers
/// it: the PLAF103. The SDK takes no product schema, so the board names its
/// model by code and the firmware checks it against `Model`.
pub const MODEL_CODE: u8 = 1;

/// The outputs' safe state, which a fault or a flash lease writes straight
/// to the data registers without asking any driver: bits set HIGH on port A
/// (the motor's STOP), then bits cleared LOW on ports A and B (the emitters
/// OFF).
pub mod safe {
    use super::pins;

    pub const PORT_A_HIGH: u32 = pins::MOTOR_MASK;
    pub const PORT_A_LOW: u32 = pins::IR_PA_BIT;
    pub const PORT_B_LOW: u32 = pins::IR_PB_BIT;
}

/// Every GPIO and ADC channel the ports use, in one place.
/// A GPIO is `port * 32 + bit` (PA13 is 13, PB3 is 35). Roles and
/// polarities are stock-derived and bench-checked where the cards say so;
/// [device layer](../../../../docs/firmware/device-layer.md) lists what else
/// is board-specific and where it lives.
pub mod pins {
    /// Motor driver inputs; both HIGH requests STOP (brake).
    pub const MOTOR_A: u32 = 13;
    pub const MOTOR_B: u32 = 14;
    /// Infrared emitter latches, LOW for OFF. PB3 is also SWD_CLK.
    pub const IR_PA: u32 = 15;
    pub const IR_PB: u32 = 32 + 3;
    /// Sensor inputs: outlet beam, food beam, feedback switch, adapter.
    pub const OUTLET: u32 = 16;
    pub const FOOD: u32 = 17;
    pub const FEEDBACK: u32 = 18;
    pub const ADAPTER: u32 = 19;
    /// Panel buttons, active LOW: reset, mic, dispense, lock (PB4–PB7).
    pub const BUTTONS: [u32; 4] = [32 + 4, 32 + 5, 32 + 6, 32 + 7];
    /// Indicators: connection, food, lock (PA0, PA2, PA4) and fault (PB22).
    pub const LED_CONNECTION: u32 = 0;
    pub const LED_FOOD: u32 = 2;
    pub const LED_LOCK: u32 = 4;
    pub const LED_FAULT: u32 = 32 + 22;
    /// Speaker amplifier enable, HIGH while playing.
    pub const AMP: u32 = 32 + 23;
    /// An active buzzer: HIGH beeps at its own pitch. Stock pulses it for
    /// 200 ms, and the feeder beeps it for each button press.
    pub const BUZZER: u32 = 25;
    /// ADC channels: motor current and the battery pack. Channel 10, the
    /// chip's own supply, is the AmebaD's, not the board's
    /// (`wellfed_amebad_sys::ADC_SUPPLY_CHANNEL`).
    pub const ADC_CURRENT: u32 = 4;
    pub const ADC_BATTERY: u32 = 5;
    /// PB1 and PB2, the pads behind those two channels: the ADC's opening
    /// shuts their digital path down so the converter reads the pin.
    pub const ADC_PADS: &[u32] = &[32 + 1, 32 + 2];

    /// The pin's bit in its port register.
    pub const fn bit(pin: u32) -> u32 {
        1 << (pin & 31)
    }

    /// Motor driver inputs PA13 and PA14 (port A); both HIGH requests STOP.
    pub const MOTOR_A_BIT: u32 = bit(MOTOR_A);
    pub const MOTOR_B_BIT: u32 = bit(MOTOR_B);
    pub const MOTOR_MASK: u32 = MOTOR_A_BIT | MOTOR_B_BIT;
    /// Infrared emitter latches, LOW for OFF: PA15 on port A and PB3 (also
    /// SWD_CLK) on port B.
    pub const IR_PA_BIT: u32 = bit(IR_PA);
    pub const IR_PB_BIT: u32 = bit(IR_PB);
}

/// Owner-approved replacement-firmware storage: 50 sectors at
/// 0x340000-0x372000. The owner approved the first 16, extended them to 32
/// (October 23, 2026) for client keys, plans and the 7-day event ring, to
/// 48 (October 26) for the lifetime statistics and plans, and to 50
/// (November 9) for the feeder's settings record. A static pass over stock
/// 1.1.15 found no fixed or bounded computed flash access in
/// 0x306000-0x3e8000, and every full dump of the research unit is blank
/// there. Clear of bank 0, of a full-size bank 1 (ends no later than
/// 0x306000) and of the stock settings (0x3e8000). The region only ever
/// grows upward, so a sector keeps its address and an older build, whose
/// bound is smaller, never reaches the sectors added after it.
/// Callers name a sector index; nothing takes a flash address.
pub const STORAGE_OFFSET: u32 = 0x34_0000;
pub const STORAGE_SECTORS: u32 = 50;
const _: () =
    assert!(STORAGE_OFFSET >= 0x30_6000 && STORAGE_OFFSET + STORAGE_SECTORS * 0x1000 <= 0x3e_8000);

/// Who owns each sector of the storage region, in one place. Each
/// store keeps its sectors to itself, so a torn record is always its own.
pub mod storage {
    use super::STORAGE_SECTORS;

    /// `count` sectors from region sector `first`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Sectors {
        pub first: u32,
        pub count: u32,
    }

    impl Sectors {
        const fn end(self) -> u32 {
            self.first + self.count
        }
    }

    /// The feed journal's slot pair.
    pub const JOURNAL: Sectors = Sectors { first: 0, count: 2 };
    /// The device's settings, those every kind keeps (lights, sound, volume
    /// and their windows); the feeder's own are [`FEEDER_SETTINGS`].
    pub const SETTINGS: Sectors = Sectors { first: 2, count: 2 };
    /// The boot confirmation record.
    pub const CONFIRM: Sectors = Sectors { first: 4, count: 1 };
    /// Wi-Fi credentials.
    pub const NET: Sectors = Sectors { first: 5, count: 2 };
    /// The random generator's seed carried across boots.
    pub const SEED: Sectors = Sectors { first: 7, count: 2 };
    /// The feeder's identity key and enrolled clients.
    pub const CLIENTS: Sectors = Sectors { first: 9, count: 2 };
    /// The feeding plans, their marks and the last claim.
    pub const PLANS: Sectors = Sectors {
        first: 11,
        count: 2,
    };
    /// Reserved for the 7-day event ring: 11 sectors of records and one
    /// being erased (sized by the resource budget).
    pub const EVENTS: Sectors = Sectors {
        first: 13,
        count: 12,
    };

    /// The update store: the anti-rollback floor, the give-way's
    /// candidate, the added and retired update keys, and bank 0's image.
    /// Not the confirmation sector, which is erased before each write, so
    /// a cut there would lose a floor kept in it.
    pub const UPDATE: Sectors = Sectors {
        first: 25,
        count: 2,
    };
    /// The time settings: the owner's zone and the NTP server. Free
    /// sectors before, so a build that predates the store never touches
    /// them.
    pub const TIME: Sectors = Sectors {
        first: 27,
        count: 2,
    };
    /// The per-feed trace ring: four 1 KiB traces a sector, the last 8 to
    /// 12 feeds. No build before it wrote these sectors.
    pub const TRACES: Sectors = Sectors {
        first: 29,
        count: 3,
    };

    /// The lifetime statistics: totals over every boot, resets by cause
    /// and erases by sector. The first sectors past the 32-sector region, so
    /// a build that predates them never reads or writes them.
    pub const STATS: Sectors = Sectors {
        first: 32,
        count: 2,
    };
    /// The portion calibration: what one portion weighs. Sectors no
    /// earlier build allocated, so a build that predates the store never
    /// touches them.
    pub const CALIBRATION: Sectors = Sectors {
        first: 34,
        count: 2,
    };

    /// The factory reset's marker: written before the reset erases
    /// anything and erased after it erased everything, so a boot that finds
    /// it finishes the reset. Blank in every earlier build.
    pub const RESET: Sectors = Sectors {
        first: 36,
        count: 1,
    };

    /// Reserved for the uplink store, the owner's server link. No build
    /// writes it yet; the factory reset and the handover from stock both
    /// clear it, as every sector off their keep lists, so a bank 0 built
    /// before the store clears it too.
    pub const UPLINK: Sectors = Sectors {
        first: 37,
        count: 3,
    };

    /// What the factory reset keeps: the feed journal (a feed is never
    /// re-run and its numbers never go back), the boot confirmation and
    /// the update store (the build in trial and the anti-rollback floor;
    /// the reset erases a corrupt update store apart, see
    /// `wellfed_core::store::update`), the generator's seed, the lifetime
    /// statistics (rewritten with only the erase counts, the hardware's
    /// wear, as a step of their own), the portion calibration (a property
    /// of the hardware), and the reset's own marker, erased last.
    ///
    /// The reset erases every other sector of the region, clients and
    /// identity first, then in region order. Whichever bank finishes a
    /// reset clears by its own build's lists, and bank 0 keeps one build
    /// for the life of the feeder: so a store a later build adds is cleared
    /// by default, and one that must survive a reset can only be added here
    /// by a build that also runs on bank 0.
    pub const FACTORY_KEPT: [Sectors; 7] =
        [JOURNAL, CONFIRM, SEED, UPDATE, STATS, CALIBRATION, RESET];

    /// The sectors the factory reset erases.
    pub const FACTORY_CLEARED_SECTORS: u32 = cleared_count(&FACTORY_KEPT);

    /// The `index`th sector the factory reset erases, in its order: the
    /// clients first, then every other sector off [`FACTORY_KEPT`].
    pub const fn factory_cleared(index: u32) -> Option<u32> {
        nth_cleared(&FACTORY_KEPT, CLIENTS, index)
    }

    /// What the handover from stock keeps: what belongs to the feeder
    /// rather than its owner (the feed journal, the generator's seed, the
    /// lifetime statistics, the portion calibration), and what it clears in
    /// steps of its own (the clients, keeping the identity; the update
    /// store's floor; the boot confirmation; the handover's own mark). The
    /// feeder may have changed hands while it ran stock, so it erases every
    /// other sector in region order, the previous owner's data among them:
    /// the settings, Wi-Fi, plans, history, traces, time zone and meal-call
    /// recording, and whatever a later build adds, for the same reason as
    /// [`FACTORY_KEPT`].
    pub const HANDOVER_KEPT: [Sectors; 8] = [
        JOURNAL,
        CONFIRM,
        SEED,
        CLIENTS,
        UPDATE,
        STATS,
        CALIBRATION,
        RESET,
    ];

    /// The sectors the handover erases.
    pub const HANDOVER_CLEARED_SECTORS: u32 = cleared_count(&HANDOVER_KEPT);

    /// The `index`th sector the handover erases, in region order.
    pub const fn handover_cleared(index: u32) -> Option<u32> {
        nth_cleared(&HANDOVER_KEPT, Sectors { first: 0, count: 0 }, index)
    }

    /// Whether `sector` is in one of `keep`.
    const fn kept(keep: &[Sectors], sector: u32) -> bool {
        let mut i = 0;
        while i < keep.len() {
            if keep[i].first <= sector && sector < keep[i].end() {
                return true;
            }
            i += 1;
        }
        false
    }

    /// The region's sectors off `keep`.
    const fn cleared_count(keep: &[Sectors]) -> u32 {
        let mut total = 0;
        let mut sector = 0;
        while sector < STORAGE_SECTORS {
            if !kept(keep, sector) {
                total += 1;
            }
            sector += 1;
        }
        total
    }

    /// The `index`th sector off `keep`: `first`'s, then the rest in region
    /// order.
    const fn nth_cleared(keep: &[Sectors], first: Sectors, index: u32) -> Option<u32> {
        if index < first.count {
            return Some(first.first + index);
        }
        let mut left = index - first.count;
        let mut sector = 0;
        while sector < STORAGE_SECTORS {
            if !kept(keep, sector) && !kept(&[first], sector) {
                if left == 0 {
                    return Some(sector);
                }
                left -= 1;
            }
            sector += 1;
        }
        None
    }

    /// The meal call: one clip the owner uploads, a header and about 8 s
    /// of ADPCM (`wellfed_core::clip`). The 48-sector region's last
    /// sectors, which no earlier build allocated.
    pub const CALL: Sectors = Sectors {
        first: 40,
        count: 8,
    };

    /// The feeder's own settings, beside the device's: the meal reminder,
    /// the desiccant reminder and the bowl mode. The first sectors past the
    /// 48-sector region, so a build that predates the store never reads or
    /// writes them; a factory reset clears them, being off its keep list.
    pub const FEEDER_SETTINGS: Sectors = Sectors {
        first: 48,
        count: 2,
    };

    /// Every allocation, in region order.
    pub const ALL: [Sectors; 17] = [
        JOURNAL,
        SETTINGS,
        CONFIRM,
        NET,
        SEED,
        CLIENTS,
        PLANS,
        EVENTS,
        UPDATE,
        TIME,
        TRACES,
        STATS,
        CALIBRATION,
        RESET,
        UPLINK,
        CALL,
        FEEDER_SETTINGS,
    ];

    const fn disjoint_and_inside() -> bool {
        let mut i = 1;
        while i < ALL.len() {
            if ALL[i - 1].end() > ALL[i].first || ALL[i].count == 0 {
                return false;
            }
            i += 1;
        }
        ALL[ALL.len() - 1].end() <= STORAGE_SECTORS
    }
    const _: () = assert!(disjoint_and_inside());

    /// The region is allocated whole, so a sector neither list keeps is
    /// always a store's, never a stray one.
    const fn covered() -> bool {
        let mut sector = 0;
        while sector < STORAGE_SECTORS {
            if !kept(&ALL, sector) {
                return false;
            }
            sector += 1;
        }
        true
    }
    const _: () = assert!(covered());
    const _: () =
        assert!(!kept(&FACTORY_KEPT, CLIENTS.first) && !kept(&FACTORY_KEPT, UPLINK.first));
    const _: () = assert!(!kept(&HANDOVER_KEPT, UPLINK.first) && !kept(&HANDOVER_KEPT, NET.first));
}
