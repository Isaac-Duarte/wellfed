//! KM4 watchdog driver against the fake register model: opening validates
//! the configuration and claims the reset scope while preserving the other
//! scope bits; a refresh needs elapsed time and owner progress; and a context
//! or register change, a frozen clock or a flash hold ends refreshes so the
//! independent watchdog expires.

mod common;

use common::*;
use wellfed_core::flash_coord::{FlashParty, FlashReply};
use wellfed_hal::sdk::{self, fake};
use wellfed_hal::supervision::watchdog::{
    Watchdog, WatchdogConfig, WatchdogFault, force_enable, force_reset,
};
use wellfed_hal_feeder::board::Board;

const CONFIG: WatchdogConfig = WatchdogConfig {
    systimer_hz: 32_000,
    clock_min_hz: 32_000,
    clock_max_hz: 32_768,
    count_id: 5,
    divisor: 63,
    min_refresh_ms: 2,
    max_progress_gap_ms: 50,
    max_expiry_ms: 126,
};

fn close_port(w: &mut Watchdog) {
    let b = Box::<Board>::default();
    assert!(b.open(2, 12, 8, 0));
    assert!(w.close(|| b.inactive()));
    assert!(!w.claimed && !w.armed && sdk::wdg_vendor() & sdk::WDG_BIT_ENABLE == 0);
    b.close();
}

fn fresh() -> Box<Watchdog> {
    fake::reset_hardware();
    Box::default()
}

#[test]
fn lifecycle_and_scope() {
    let mut w = fresh();
    hw(|h| {
        h.hs_control = 0x125;
        h.lp_control = 0x2000; // LP reset-scope bit 13 must be preserved.
    });

    assert!(w.open(&CONFIG));
    assert!(w.claimed && w.armed && hw(|h| h.watchdog_init_calls) == 1);
    assert_eq!(hw(|h| h.hs_control), 0x125 | sdk::HS_WDTRST_OPT);
    assert_eq!(hw(|h| h.lp_control), 0x2000 | sdk::LP_WDT_RESET_HS_ALL);
    assert!(hw(|h| h.watchdog_period) == 4032 && hw(|h| h.interrupts) == 0);

    assert!(w.checkpoint(1));
    assert_eq!(hw(|h| h.watchdog_refreshes), 0); // No elapsed time, no refresh.
    fake::advance(32, false);
    assert!(w.checkpoint(2));
    assert_eq!(hw(|h| h.watchdog_refreshes), 0);
    fake::advance(32, false);
    assert!(w.checkpoint(3));
    assert!(hw(|h| h.watchdog_refreshes) == 1 && hw(|h| h.watchdog_ticks) == 0);

    hw(|h| {
        h.hs_control |= 0x200;
        h.lp_control |= 0x400;
    });
    close_port(&mut w);
    assert!(hw(|h| h.hs_control) == 0x325 && hw(|h| h.lp_control) == 0x2400);
}

#[test]
fn invalid_configuration_and_resources() {
    let mut w = fresh();
    let mut other = Box::<Watchdog>::default();
    assert!(!w.open(&WatchdogConfig {
        clock_min_hz: 0,
        ..CONFIG
    }));

    for bad in [
        WatchdogConfig {
            clock_max_hz: 1000,
            ..CONFIG
        },
        WatchdogConfig {
            count_id: 12,
            ..CONFIG
        },
        WatchdogConfig {
            divisor: 0,
            ..CONFIG
        },
        WatchdogConfig {
            max_progress_gap_ms: 62,
            ..CONFIG
        },
        WatchdogConfig {
            min_refresh_ms: 0,
            ..CONFIG
        },
        WatchdogConfig {
            min_refresh_ms: 50,
            ..CONFIG
        },
        WatchdogConfig {
            max_expiry_ms: 125,
            ..CONFIG
        },
        WatchdogConfig {
            divisor: 64,
            ..CONFIG
        },
        WatchdogConfig {
            max_expiry_ms: 0,
            ..CONFIG
        },
    ] {
        assert!(!bad.valid());
    }

    assert_eq!(hw(|h| h.watchdog_init_calls), 0);
    hw(|h| h.interrupts = 1);
    assert!(!w.open(&CONFIG) && hw(|h| h.interrupts) == 1);
    hw(|h| h.interrupts = 0);

    hw(|h| h.watchdog_vendor = sdk::WDG_BIT_ENABLE);
    assert!(!w.open(&CONFIG) && !w.claimed);
    hw(|h| h.watchdog_vendor = 0);

    assert!(w.open(&CONFIG));
    assert!(!other.open(&CONFIG) && !other.claimed);
    close_port(&mut w);
}

#[test]
fn progress_deadline_and_replay_latch() {
    let mut w = fresh();
    assert!(w.open(&CONFIG));

    fake::advance(49 * 32, false);
    assert!(w.check() && hw(|h| h.watchdog_refreshes) == 0);
    fake::advance(32, false);
    assert!(!w.checkpoint(1));
    assert!(w.fault == WatchdogFault::Progress && w.armed);
    assert!(!w.checkpoint(2) && hw(|h| h.watchdog_refreshes) == 0);
    close_port(&mut w);

    assert!(w.open(&CONFIG));
    fake::advance(64, false);
    assert!(w.checkpoint(u64::MAX));
    assert!(!w.checkpoint(u64::MAX));
    assert_eq!(w.fault, WatchdogFault::Progress);
    close_port(&mut w);
}

#[test]
fn independent_expiry_with_frozen_clock_and_masked_irqs() {
    let mut w = fresh();
    assert!(w.open(&CONFIG));
    for token in 1..=10 {
        assert!(w.checkpoint(token));
        fake::watchdog_advance(400); // SYSTIMER deliberately frozen.
    }

    assert!(hw(|h| h.watchdog_refreshes) == 0 && hw(|h| h.raw_ticks) == 0);

    hw(|h| h.interrupts = 1);
    fake::watchdog_advance(32);
    assert!(hw(|h| h.watchdog_expired
        && h.watchdog_reset_requests == 1
        && h.timer_irqs == 0));
    hw(|h| h.interrupts = 0);
    assert!(!w.check() && w.fault == WatchdogFault::Register);
    close_port(&mut w); // Model reports a reset request; it does not reboot the host.
}

#[test]
fn context_and_register_changes() {
    for change in 0..7 {
        let mut w = fresh();
        assert!(w.open(&CONFIG));

        hw(|h| match change {
            0 => h.ipsr = 19,
            1 => h.interrupts = 1,
            2 => h.watchdog_vendor ^= sdk::WDG_BIT_RST_MODE,
            3 => h.watchdog_vendor ^= 1,
            4 => h.hs_control &= !sdk::HS_WDTRST_OPT,
            5 => h.lp_control &= !sdk::LP_WDT_RESET_HS_ALL,
            _ => h.watchdog_vendor &= !sdk::WDG_BIT_ENABLE,
        });

        let mask = hw(|h| h.interrupts);
        assert!(!w.checkpoint(1));
        assert!(hw(|h| h.interrupts) == mask && hw(|h| h.watchdog_refreshes) == 0);
        assert_eq!(
            w.fault,
            if change < 2 {
                WatchdogFault::Context
            } else {
                WatchdogFault::Register
            }
        );

        hw(|h| {
            h.ipsr = 0;
            h.interrupts = 0;
        });
        close_port(&mut w);
    }
}

#[test]
fn setup_rollback_and_retained_cleanup() {
    let mut w = fresh();
    hw(|h| h.watchdog_scope_blocked = true);
    assert!(!w.open(&CONFIG));
    assert!(!w.claimed && !w.armed && w.fault == WatchdogFault::Setup);

    hw(|h| {
        h.watchdog_scope_blocked = false;
        h.watchdog_enable_corrupt = true;
        h.watchdog_disable_blocked = true;
    });
    assert!(!w.open(&CONFIG));
    assert!(w.claimed && w.armed);
    assert!(!w.close(|| false));
    hw(|h| {
        h.watchdog_disable_blocked = false;
        h.watchdog_enable_corrupt = false;
    });
    close_port(&mut w);

    assert!(w.open(&CONFIG));
    let b = Box::<Board>::default();
    assert!(b.open(2, 12, 8, 0));
    let _authority = flash_attach(&b);
    assert!(b.optics(true));
    assert!(!w.close(|| b.inactive()) && w.claimed);
    assert!(b.optics(false));
    hw(|h| h.watchdog_disable_blocked = true);
    assert!(!w.close(|| b.inactive()) && w.claimed && w.armed);
    hw(|h| h.watchdog_disable_blocked = false);
    assert!(w.close(|| b.inactive()));

    assert!(w.open(&CONFIG));
    hw(|h| h.watchdog_scope_blocked = true);
    assert!(!w.close(|| b.inactive()));
    assert!(w.claimed && !w.armed); // Disabled, but restoration remains unresolved.
    hw(|h| h.watchdog_scope_blocked = false);
    assert!(w.close(|| b.inactive()));
    b.close();
}

#[test]
fn hardware_counter_wrap_and_clock_fault() {
    let mut w = fresh();
    hw(|h| h.raw_ticks = u32::MAX - 31);
    assert!(w.open(&CONFIG));
    fake::advance(64, false);
    assert!(w.checkpoint(1) && hw(|h| h.watchdog_refreshes) == 1);

    hw(|h| h.raw_ticks -= 1);
    assert!(!w.check() && w.fault == WatchdogFault::Clock);
    close_port(&mut w);
}

#[test]
fn flash_hold_cannot_refresh_or_extend_progress() {
    let mut w = fresh();
    let mut c = flash_ready();
    assert!(w.open(&CONFIG));

    fake::advance(64, false);
    // SAFETY: null or a live coordinator.
    assert!(!unsafe { w.checkpoint_if_ready(1, core::ptr::null()) });
    // SAFETY: null or a live coordinator.
    assert!(unsafe { w.checkpoint_if_ready(1, &*c) });
    assert!(hw(|h| h.watchdog_refreshes) == 1 && w.progress_at == 2);

    assert_eq!(c.request(FlashParty::Peer, 29, 1, 0), FlashReply::Accepted);
    for n in 2..51 {
        fake::advance(32, false);
        // SAFETY: null or a live coordinator.
        assert!(unsafe { w.checkpoint_if_ready(n, &*c) });
        assert!(w.progress == 1 && w.progress_at == 2 && hw(|h| h.watchdog_refreshes) == 1);
    }

    fake::advance(32, false);
    // SAFETY: null or a live coordinator.
    assert!(!unsafe { w.checkpoint_if_ready(51, &*c) });
    assert!(w.fault == WatchdogFault::Progress && hw(|h| h.watchdog_refreshes) == 1);
    close_port(&mut w);
}

#[test]
fn sdk_init_action_bits_read_back_clear() {
    // WDG_Init writes CLEAR (24) and ISR_CLEAR (31); both are write-1 actions.
    let mut w = fresh();
    assert!(w.open(&CONFIG));
    let vendor = hw(|h| h.watchdog_vendor);
    assert!(vendor & sdk::WDG_BIT_ISR_CLEAR == 0 && vendor & (1 << 24) == 0);
    assert!(vendor & sdk::WDG_BIT_ENABLE != 0 && vendor & sdk::WDG_BIT_RST_MODE != 0);
    close_port(&mut w);
}

/// A watchdog that will not open (its enable bit already set, or its scope
/// held) still resets the platform: `force_reset` programs the scope and
/// the counter as `open` does and leaves it unfed, so the reset comes
/// within the configured expiry and KM0 counts the boot.
#[test]
fn a_watchdog_that_will_not_open_still_resets() {
    let mut w = fresh();
    hw(|h| h.watchdog_vendor |= sdk::WDG_BIT_ENABLE);
    assert!(!w.open(&CONFIG) && !w.claimed);

    assert!(force_reset(&CONFIG));
    assert!(hw(|h| h.watchdog_period) == 4032 && hw(|h| h.watchdog_ticks) == 0);
    assert!(sdk::hs_platform_para() & sdk::HS_WDTRST_OPT != 0);
    assert!(sdk::lp_km0_ctrl() & sdk::LP_WDT_RESET_HS_ALL != 0);
    assert_eq!(hw(|h| h.interrupts), 0, "interrupts as they were");
    fake::watchdog_advance(4031);
    assert_eq!(hw(|h| h.watchdog_reset_requests), 0);
    fake::watchdog_advance(1);
    assert_eq!(hw(|h| h.watchdog_reset_requests), 1);

    // A setup that did not verify, the claim retained: the same reset.
    let mut w = fresh();
    hw(|h| {
        h.watchdog_enable_corrupt = true;
        h.watchdog_disable_blocked = true;
    });
    assert!(!w.open(&CONFIG) && w.claimed);
    hw(|h| {
        h.watchdog_enable_corrupt = false;
        h.watchdog_disable_blocked = false;
    });
    assert!(force_reset(&CONFIG));
    fake::watchdog_advance(4032);
    assert_eq!(hw(|h| h.watchdog_reset_requests), 1);

    // An invalid configuration touches nothing.
    fake::reset_hardware();
    assert!(!force_reset(&WatchdogConfig {
        divisor: 0,
        ..CONFIG
    }));
    assert!(hw(|h| h.watchdog_init_calls) == 0 && sdk::wdg_vendor() & sdk::WDG_BIT_ENABLE == 0);
}

/// An enable that does not read back while the counter runs: the forced
/// reset reports it did not take, and its retries only enable the counter
/// again, never programming it, so its count runs on and the reset still
/// comes within the configured expiry.
#[test]
fn a_forced_reset_whose_enable_does_not_read_back_still_resets() {
    let mut w = fresh();
    hw(|h| h.watchdog_vendor |= sdk::WDG_BIT_ENABLE);
    assert!(!w.open(&CONFIG));
    hw(|h| h.watchdog_enable_hidden = true);

    assert!(!force_reset(&CONFIG));
    assert_eq!(hw(|h| h.watchdog_init_calls), 1);
    // Retries a quarter of the expiry apart, as the feeder task's would be.
    for _ in 0..3 {
        fake::watchdog_advance(1000);
        assert!(!force_enable());
        assert_eq!(hw(|h| h.watchdog_reset_requests), 0);
    }
    fake::watchdog_advance(1032);
    assert_eq!(hw(|h| h.watchdog_reset_requests), 1);
    assert_eq!(hw(|h| h.watchdog_init_calls), 1, "programmed once");
}
