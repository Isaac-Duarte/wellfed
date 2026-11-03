//! What one direct session tells the path router: whether it is open, the
//! newest event it knows of on the feeder, and the feeds this phone's key
//! asked for as the feeder's history records them. The session writes it as
//! it runs; the router reads it and hears each change through its listener.
//!
//! The listener is called with no lock held here, so the router may read
//! the link from inside it.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::mpsc;
use wellfed_client::history::{Delivered, Feed};
use wellfed_model::Source;

use crate::route::core::Seen;
use crate::session::Command;

/// How many of this phone's feed ends a link keeps for a router that
/// attaches late: more than an uncertain feed waits behind.
const SEEN_KEPT: usize = 64;

/// A change the router hears.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LinkNews {
    Opened,
    Closed,
    /// One of this phone's feed ends, now in [`DirectLink::seen`].
    Fed,
}

pub(crate) type Listener = Arc<dyn Fn(LinkNews) + Send + Sync>;

/// One direct session, as the router reaches it.
pub(crate) struct DirectLink {
    /// The feeder's identity, and as lowercase hex.
    pub(crate) identity: [u8; 32],
    pub(crate) feeder: String,
    /// The session's commands.
    pub(crate) commands: mpsc::UnboundedSender<Command>,
    /// This phone's key's first 4 bytes, as events name a client.
    phone: [u8; 4],
    state: Mutex<LinkState>,
}

#[derive(Default)]
struct LinkState {
    open: bool,
    newest: Option<u64>,
    seen: Vec<Seen>,
    listener: Option<Listener>,
}

impl DirectLink {
    pub(crate) fn new(
        feeder: [u8; 32],
        commands: mpsc::UnboundedSender<Command>,
        phone: &[u8; 32],
    ) -> Self {
        let mut fingerprint = [0; 4];
        fingerprint.copy_from_slice(&phone[..4]);
        Self {
            identity: feeder,
            feeder: wellfed_formats::hex::encode(&feeder),
            commands,
            phone: fingerprint,
            state: Mutex::new(LinkState::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, LinkState> {
        // A panic while holding it leaves plain values, still readable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn is_open(&self) -> bool {
        self.state().open
    }

    /// The newest event the session knows the feeder holds.
    pub(crate) fn newest(&self) -> Option<u64> {
        self.state().newest
    }

    /// This phone's feed ends the session has read.
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.state().seen.clone()
    }

    /// Hear every change from now on.
    pub(crate) fn listen(&self, listener: Listener) {
        self.state().listener = Some(listener);
    }

    fn tell(&self, news: LinkNews) {
        let listener = self.state().listener.clone();
        if let Some(listener) = listener {
            listener(news);
        }
    }

    pub(crate) fn opened(&self) {
        self.state().open = true;
        self.tell(LinkNews::Opened);
    }

    pub(crate) fn closed(&self) {
        self.state().open = false;
        self.tell(LinkNews::Closed);
    }

    /// The feeder's newest event is at least `seq`.
    pub(crate) fn event_at(&self, seq: Option<u64>) {
        let mut state = self.state();
        state.newest = state.newest.max(seq);
    }

    /// A feed's end the history gave; kept and told when this phone's key
    /// asked for it.
    pub(crate) fn fed(&self, feed: &Feed) {
        self.event_at(Some(feed.seq));
        let mine = matches!(feed.by, Some(Source::Client { client }) if client.0 == self.phone);
        if !mine {
            return;
        }
        let completed = match feed.delivered {
            Delivered::Counted(portions) => portions,
            Delivered::Uncertain { confirmed, .. } => confirmed,
        };
        let seen = Seen {
            seq: feed.seq,
            feed: feed.feed,
            requested: feed.requested,
            completed,
            outcome: feed.outcome,
            reason: feed.reason,
        };
        {
            let mut state = self.state();
            if state.seen.iter().any(|kept| kept.seq == seen.seq) {
                return;
            }
            if state.seen.len() >= SEEN_KEPT {
                state.seen.remove(0);
            }
            state.seen.push(seen);
        }
        self.tell(LinkNews::Fed);
    }
}
