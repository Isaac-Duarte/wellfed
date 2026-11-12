//! The fake's supervision timer: an update sets its flag whatever the
//! interrupt enable says, and reaches the handler only through the enable.

use core::ffi::c_void;

use wellfed_amebad_sys::{self as sdk, Irq, fake};

extern "C" fn count(_context: *mut c_void) -> u32 {
    sdk::tim2_int_clear();
    0
}

fn running(period: u32) {
    fake::reset_hardware();
    sdk::tim2_init(period);
    sdk::tim2_int_clear();
    assert!(sdk::irq_register(Irq::Timer2, count, 4));
    sdk::irq_enable(Irq::Timer2, 4);
    sdk::tim2_cmd(true);
}

fn irqs() -> u32 {
    fake::with(|h| h.timer_irqs)
}

#[test]
fn an_update_with_its_interrupt_disabled_raises_nothing() {
    running(99);
    fake::advance(100, true);
    assert_eq!(irqs(), 0);
    assert_ne!(fake::with(|h| h.timer.sr) & sdk::TIM_IT_UPDATE, 0);

    // Enabling it over the flag still set raises the line once.
    sdk::tim2_int_config(true);
    assert_eq!(irqs(), 1);
    fake::advance(100, true);
    assert_eq!(irqs(), 2);
}
