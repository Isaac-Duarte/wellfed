//! The feeder's synthetic startup check, which the SDK runtime's startup
//! task calls (`wellfed_startup_check`) before it lets any other task run:
//! one single-portion feed through the feeder's core logic on a
//! memory-only motor (`wellfed_core_feeder::feed::diagnostic`). The SDK
//! may not depend on the feeder's logic, so it declares the hook and this
//! crate, which every PLAF103 firmware links, defines it.

/// The SDK runtime's startup check. Its signature must match the runtime's
/// `extern "Rust"` declaration in `wellfed-sdk`'s `runtime.rs` (the linker
/// does not check it); the assertion below pins it.
#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub fn wellfed_startup_check() -> bool {
    wellfed_core_feeder::feed::diagnostic::run()
}

#[cfg(target_os = "none")]
const _: fn() -> bool = wellfed_startup_check;
