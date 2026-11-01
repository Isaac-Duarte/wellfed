//! `console`: the server's full-screen console, in another process.
//!
//! It draws a [`Watched`] server and nothing else: it holds no client, so
//! `q` and Ctrl+C can only leave, and the server and its job run on. A job's
//! question shows as on the server's own console, with the `answer` line
//! that answers it. The server's own console-only lines are not on the
//! socket, so the log here holds job output only.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tokio::runtime::Runtime;
use wellfed_bench_server::{Client, Watched};
use wellfed_formats::bench::live::BenchState;

use super::Feed;

/// The footer's keys.
pub(super) const KEYS: &str =
    "q or Ctrl+C leaves; the server keeps running · PgUp/PgDn scroll · End follows";
/// The footer while the server does not answer, over its last state.
pub(super) const NOT_ANSWERING: &str =
    "server not answering, retrying (last state shown); q leaves";

/// Run the console until `q`, Ctrl+C or a signal, or refuse with no terminal
/// or no server.
pub(crate) fn run(root: &Path, runtime: &Runtime) -> anyhow::Result<u8> {
    if !super::available(false) {
        anyhow::bail!(
            "`console` draws on a terminal, and this has none; over ssh, use `ssh -t HOST wellfed-bench console`"
        );
    }
    let tty = super::tty()?;
    let watched = runtime.block_on(Watched::start(Client::new(root)))?;

    let left = Arc::new(AtomicBool::new(false));
    runtime.block_on(leave_on_signals(&left))?;
    super::run(tty, &mut Attached { watched, left })?;
    Ok(0)
}

/// Leave on a signal too, so the terminal gets out of raw mode and the
/// alternate screen rather than being left in them. These only end this
/// process: its server is another one.
async fn leave_on_signals(left: &Arc<AtomicBool>) -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let left = Arc::clone(left);
    tokio::spawn(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
        left.store(true, Ordering::SeqCst);
    });
    Ok(())
}

struct Attached {
    watched: Watched,
    left: Arc<AtomicBool>,
}

impl Feed for Attached {
    fn keys(&self) -> &'static str {
        KEYS
    }

    fn state(&self) -> BenchState {
        self.watched.state()
    }

    fn line(&mut self) -> Option<String> {
        self.watched.line()
    }

    fn notice(&self, _at: Instant) -> Option<&'static str> {
        (!self.watched.answering()).then_some(NOT_ANSWERING)
    }

    fn quit_pressed(&mut self, _at: Instant) {
        self.left.store(true, Ordering::SeqCst);
    }

    fn finished(&self) -> bool {
        self.left.load(Ordering::SeqCst)
    }
}
