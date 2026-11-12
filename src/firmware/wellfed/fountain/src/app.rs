//! The entry the SDK's `example_entry` calls, and the beat task.

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use wellfed_hal::sdk;

use crate::boot;

/// The start, one record, and a formatted beat; the build's stack audit
/// checks the bound.
const BOOT_STACK_WORDS: u16 = 1024;

/// Beats so far; only the beat task counts.
static BEATS: AtomicU32 = AtomicU32::new(0);

/// Called once by the SDK's `example_entry`, before the scheduler starts.
/// The pump is latched off first, before anything else runs.
#[unsafe(no_mangle)]
#[expect(
    unreachable_pub,
    reason = "a C export: the SDK's entry calls it by name"
)]
pub extern "C" fn wellfed_app_start() {
    boot::start(wellfed_hal::startup::snapshot);

    // SAFETY: the step ignores its context, which is null, and is sound to
    // call again and again from the new task.
    let created = unsafe {
        sdk::task_create(
            c"wellfed-boot",
            BOOT_STACK_WORDS,
            1,
            beat_task,
            ptr::null_mut(),
        )
    };
    if !created {
        sdk::log(c"PLBOOT1 error task=uncreated\r\n");
    }
}

unsafe extern "C" fn beat_task(_: *mut c_void) -> u32 {
    let count = BEATS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    boot::beat(count);
    boot::BEAT_MS
}
