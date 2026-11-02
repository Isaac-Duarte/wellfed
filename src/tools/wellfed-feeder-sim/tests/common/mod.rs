//! What several of the simulator's test binaries share and the library
//! cannot hold: tokio's paused clock is a dev-dependency feature.

use std::future::Future;

use tokio::task::LocalSet;

/// A test on tokio's paused clock, which jumps ahead whenever every task
/// waits, so the 30 s relock and 90 s idle limit pass at once.
pub fn paused(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(LocalSet::new().run_until(test));
}
