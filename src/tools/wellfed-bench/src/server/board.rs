//! The relay board the server holds for its life, shared between the job
//! that switches it and the live state that shows it.
//!
//! Every set and query goes through one lock, so the state's relays are the
//! board's last answers: nothing else can switch a relay while the server
//! holds the port, except the board's own front buttons, which the query
//! after every job picks up.

use std::sync::{Arc, Mutex, MutexGuard};

use wellfed_bench_formats::live::{BoardReading, PowerSource};
use wellfed_bench_formats::now;
use wellfed_bench_formats::progress::RelayState;
use wellfed_bench_icd::Channel;
use wellfed_relay::{Board, Source, Status, Switch};

use crate::server::state::Store;

/// The board's readings as the live state carries them, read now.
pub fn reading(status: &Status) -> BoardReading {
    let t = status.telemetry;
    BoardReading {
        vbus_mv: t.vbus_mv,
        current_ma: t.current_ma,
        source: match t.source {
            Source::Host => PowerSource::Host,
            Source::Aux => PowerSource::Aux,
        },
        fault: t.fault,
        tx_mv: t.tx_mv,
        alive: t.alive(),
        cuts: status.cuts,
        at: now(),
    }
}

#[derive(Clone)]
pub(crate) struct SharedBoard {
    board: Arc<Mutex<Box<dyn Board>>>,
    store: Store,
}

impl SharedBoard {
    pub(crate) fn new(board: Box<dyn Board>, store: Store) -> Self {
        Self {
            board: Arc::new(Mutex::new(board)),
            store,
        }
    }

    /// Ask the board for its channels and readings, in one request.
    pub(crate) fn query(&self) -> wellfed_relay::Result<()> {
        self.status().map(drop)
    }

    /// The board's status, published as it answered.
    fn status(&self) -> wellfed_relay::Result<Status> {
        let status = self.lock().status()?;
        for channel in Channel::ALL {
            self.publish(channel.number(), status.channels.get(channel));
        }
        let reading = reading(&status);
        self.store.update(|state| state.board = Some(reading));
        Ok(status)
    }

    fn lock(&self) -> MutexGuard<'_, Box<dyn Board>> {
        // A job that panicked mid-command leaves a board whose state the
        // next query reads again.
        super::lock(&self.board)
    }

    fn publish(&self, channel: u8, on: bool) {
        self.store.update(|state| {
            state
                .relays
                .insert(channel.to_string(), RelayState { on, at: now() });
        });
    }
}

impl Board for SharedBoard {
    fn set(&mut self, channel: u8, switch: Switch) -> wellfed_relay::Result<()> {
        self.lock().set(channel, switch)?;
        self.publish(channel, switch.is_on());
        Ok(())
    }

    fn status(&mut self) -> wellfed_relay::Result<Status> {
        SharedBoard::status(self)
    }
}
