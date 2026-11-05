//! What a simulator's tests share, whatever the device: a current-thread
//! runtime with a `LocalSet` ([`local`]), the process-wide hold two
//! simulators of one process need ([`Alone`]), a relay that can stall like
//! a device that lost power ([`Relay`]), and a generator whose outputs
//! repeat from run to run ([`counter`]).

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::LocalSet;

use crate::stores::Random;

/// Run `test` on a current-thread runtime, as a simulator's tasks need.
pub fn local(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime builds");
    runtime.block_on(LocalSet::new().run_until(test));
}

/// The update stage's mailbox and the interlock are process-wide statics,
/// as on the device, so simulators on two threads of one process (`cargo
/// test` runs tests on threads) would take each other's requests. A thread
/// starting a simulator holds the process until its last one stops; the
/// simulators of one thread share the hold, so a test may run two.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

thread_local! {
    /// This thread's hold on the process, and how many of its simulators
    /// share it.
    static HOLD: RefCell<Option<(MutexGuard<'static, ()>, usize)>> = const { RefCell::new(None) };
}

/// One simulator's share of its thread's hold on the process: whoever
/// starts a simulator takes one and keeps it while the simulator runs.
#[derive(Debug)]
pub struct Alone;

impl Alone {
    pub fn take() -> Self {
        HOLD.with_borrow_mut(|hold| match hold {
            Some((_, simulators)) => *simulators += 1,
            None => {
                // A test that failed while holding it stopped its simulator.
                let guard = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
                *hold = Some((guard, 1));
            }
        });
        Self
    }
}

impl Drop for Alone {
    fn drop(&mut self) {
        // At thread exit the hold is already gone with the thread's locals.
        let _ = HOLD.try_with(|hold| {
            let mut hold = hold.borrow_mut();
            if let Some((_, simulators)) = hold.as_mut() {
                *simulators -= 1;
                if *simulators == 0 {
                    *hold = None;
                }
            }
        });
    }
}

/// A relay in front of a simulator that can stall: while paused it stops
/// moving bytes either way but keeps both sockets open, as a device that
/// lost power does (no reset reaches the client). It still takes in what
/// arrives, so a test sees a request waiting ([`Relay::holding_requests`]),
/// and resuming delivers what waited, in order. It can also hold back only
/// what the device sends (an answer lost on its way back), and cut every
/// connection it carries.
#[derive(Debug)]
pub struct Relay {
    pub address: SocketAddr,
    to_device: watch::Sender<bool>,
    to_client: Arc<watch::Sender<bool>>,
    cut: watch::Sender<u64>,
    hold: Hold,
}

/// What clients send on its way to the device: how much more passes
/// before the device's answers are held back, once
/// [`Relay::hold_answers_after`] set it, and how much the relay has taken
/// in but not yet delivered.
#[derive(Clone, Debug)]
struct Hold {
    left: Arc<Mutex<Option<usize>>>,
    answers: Arc<watch::Sender<bool>>,
    waiting: Arc<AtomicUsize>,
    /// Every byte delivered to the device so far.
    passed: Arc<AtomicUsize>,
}

impl Hold {
    /// Count `n` bytes about to go to the device; when they reach the
    /// budget, hold the answers back before they are sent. A client that
    /// waits for each answer before its next request (an install's steps)
    /// had every earlier one answered, so none slips through after.
    fn sending(&self, n: usize) {
        self.passed.fetch_add(n, Ordering::SeqCst);
        let mut left = self.left.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(budget) = left.as_mut() {
            if n >= *budget {
                *left = None;
                self.answers.send_replace(true);
            } else {
                *budget -= n;
            }
        }
    }
}

impl Relay {
    pub async fn start(target: SocketAddr) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the relay's port");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        let (to_device, device_paused) = watch::channel(false);
        let (to_client, client_paused) = watch::channel(false);
        let to_client = Arc::new(to_client);
        let (cut, cuts) = watch::channel(0);
        let hold = Hold {
            left: Arc::new(Mutex::new(None)),
            answers: Arc::clone(&to_client),
            waiting: Arc::new(AtomicUsize::new(0)),
            passed: Arc::new(AtomicUsize::new(0)),
        };
        let holding = hold.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(device) = TcpStream::connect(target).await else {
                    continue;
                };
                let (client_in, client_out) = client.into_split();
                let (device_in, device_out) = device.into_split();
                let forward = relay(
                    client_in,
                    device_out,
                    device_paused.clone(),
                    Some(holding.clone()),
                );
                let back = relay(device_in, client_out, client_paused.clone(), None);
                // Cuts from now on end this connection.
                let mut this = cuts.clone();
                this.mark_unchanged();
                tokio::spawn(until_cut(forward, this.clone()));
                tokio::spawn(until_cut(back, this));
            }
        });
        Self {
            address,
            to_device,
            to_client,
            cut,
            hold,
        }
    }

    pub fn pause(&self) {
        self.to_device.send_replace(true);
        self.to_client.send_replace(true);
    }

    pub fn resume(&self) {
        self.to_device.send_replace(false);
        self.to_client.send_replace(false);
    }

    /// Deliver what the client sends, but hold back what the device sends
    /// until [`Self::resume`].
    pub fn hold_answers(&self) {
        self.to_client.send_replace(true);
    }

    /// Deliver `bytes` more of what clients send, then hold back what the
    /// device sends until [`Self::resume`]: the request that crosses the
    /// budget reaches the device, its answer does not. The relay decides as
    /// the bytes pass, so a test holds a transfer part-way through without
    /// racing it.
    pub fn hold_answers_after(&self, bytes: usize) {
        let mut left = self
            .hold
            .left
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *left = Some(bytes);
    }

    /// How many bytes clients have sent the device through the relay: the
    /// difference across one request is its size on the wire, for a
    /// [`Self::hold_answers_after`] budget that lets it through and holds
    /// the next.
    pub fn passed(&self) -> usize {
        self.hold.passed.load(Ordering::SeqCst)
    }

    /// Whether the device's answers are held back now.
    pub fn holding_answers(&self) -> bool {
        *self.to_client.borrow()
    }

    /// Whether bytes a client sent wait in the relay for the device. While
    /// paused, that is a request the client has sent and the device has
    /// not seen: it has left the client, so the client cannot take it back.
    pub fn holding_requests(&self) -> bool {
        self.hold.waiting.load(Ordering::SeqCst) > 0
    }

    /// Close every connection the relay carries now, dropping what it held
    /// back; later connections pass as before.
    pub fn cut(&self) {
        self.cut.send_modify(|cuts| *cuts += 1);
    }
}

/// Run `copy` until the relay's connections are cut. A cut wins over
/// bytes waiting to be copied, so what it held back is dropped.
async fn until_cut(copy: impl Future<Output = ()>, mut cuts: watch::Receiver<u64>) {
    tokio::select! {
        biased;
        _ = cuts.changed() => {}
        () = copy => {}
    }
}

/// Copy `from` to `to`, holding what it read while paused, and counting
/// what goes to the device against `hold`'s budget.
async fn relay(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    mut paused: watch::Receiver<bool>,
    hold: Option<Hold>,
) {
    let mut buffer = [0u8; 2048];
    loop {
        // A read error ends the copy as the end of the stream does.
        let n = from.read(&mut buffer).await.unwrap_or(0);
        let _waiting = hold.as_ref().map(|hold| Waiting::new(&hold.waiting, n));

        // An end waits too, so a pause hides a closing client as well.
        if paused.wait_for(|paused| !paused).await.is_err() || n == 0 {
            return;
        }
        if let Some(hold) = &hold {
            hold.sending(n);
        }
        if to.write_all(&buffer[..n]).await.is_err() {
            return;
        }
    }
}

/// Bytes the relay has read and not yet delivered, counted until they are
/// written or dropped: a cut ends the copy part-way.
struct Waiting<'a> {
    count: &'a AtomicUsize,
    n: usize,
}

impl<'a> Waiting<'a> {
    fn new(count: &'a AtomicUsize, n: usize) -> Self {
        count.fetch_add(n, Ordering::SeqCst);
        Self { count, n }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(self.n, Ordering::SeqCst);
    }
}

/// A generator that never repeats within a run: identities, ephemerals and
/// invite secrets all come from it. The count sits clear of the bits X25519
/// clamps (byte 0's low three, byte 31's top two), so every output is a
/// distinct key.
pub fn counter(seed: u8) -> impl Fn() -> Random {
    let next = Rc::new(Cell::new(0u32));
    move || {
        let next = Rc::clone(&next);
        Box::new(move || {
            let n = next.get().wrapping_add(1);
            next.set(n);
            let mut out = [seed; 32];
            out[8..12].copy_from_slice(&n.to_le_bytes());
            Some(out)
        })
    }
}
