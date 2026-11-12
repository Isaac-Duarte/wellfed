//! The device API over tokio: [`wellfed_client::connection`]'s sans-IO
//! session driven over a real TCP connection. Every session rule
//! (keepalive, silence, answers, the event cursor) is the client crate's;
//! this crate only moves bytes and keeps the clock.
//!
//! - [`locate`] finds a feeder by its identity over mDNS, or takes the
//!   address it is given. [`Browse`] lists every feeder announcing itself,
//!   for the apps that have no mDNS of their own (the Linux desktop).
//! - [`Connection`] is an open session. [`Connection::ask`] sends one
//!   request and waits for its answer; events, gaps and samples that arrive
//!   meanwhile stay queued for [`Connection::next`] rather than being lost.
//!   A caller that keeps several requests outstanding (the server's feeder
//!   task) queues each with [`Connection::request`] and takes answers and
//!   pushed messages alike from [`Connection::receive`], matching answers
//!   by id: the feeder reads one request per client per pass and answers
//!   in the order they arrive.
//! - [`join`] redeems an invite, and [`Recovery`] runs the recovery window.
//! - [`update::push`] drives a signed update until bank 1 is activated,
//!   over the CLI's own session or the server's feeder task.
//!
//! Every method is cancel-safe, so a caller can `select!` on
//! [`Connection::next`] beside its own work: reads keep nothing in flight,
//! and a write cut short stays queued and finishes before the next one. A
//! cancelled [`Connection::ask`] may still have sent its request; its
//! answer is then dropped.
//!
//! Randomness is the caller's: request ids and ephemeral keys come in as
//! arguments, so a server can draw them from its own generator and tests
//! can fix them. A request that times out may still have run. A resend
//! under the same id gets the first answer only while the feeder still
//! remembers the id (its last 32 writes of the key, this boot), so a
//! caller never resends a feed it didn't hear answered, and settles it
//! from the history ([`wellfed_client::api`]).

#![forbid(unsafe_code)]

pub mod update;

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout, timeout_at};
use wellfed_client::api::{Invite, Join, JoinStep};
use wellfed_client::connection::{self, Event, Opened, Recovery as Window};
pub use wellfed_client::connection::{Cursor, KEEPALIVE_ID, Resume, Timing};
use wellfed_model::setup::Reply;
use wellfed_model::{
    Answer, ClientRole, DeviceKind, Hello, Joined, Model, Op, OpenRefusal, Pushed, Refusal,
};
use wellfed_proto::api::{PORT, RequestId, SERVICE};
use wellfed_session::hex::Lower;
use wellfed_session::noise::Keypair;

/// mDNS answers on a LAN arrive within a second or two.
pub const DISCOVERY: Duration = Duration::from_secs(6);
pub const CONNECT: Duration = Duration::from_secs(5);
/// A request is answered at once, or after its store write.
pub const ANSWER: Duration = Timing::DEFAULT.answer;
/// Hearing nothing for this long, the feeder is gone.
pub const SILENCE: Duration = Timing::DEFAULT.silence;

/// What a device answers an op of another kind's module.
const UNAVAILABLE: Answer = Answer::Refused(Refusal::Unavailable);
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: not an address or address:port")]
    Address(String),
    #[error("mDNS: {0}")]
    Mdns(#[from] mdns_sd::Error),
    #[error("no feeder {0} answered over mDNS; pass its address")]
    NotFound(String),
    #[error("connect to {0}: timed out")]
    ConnectTimeout(SocketAddr),
    #[error("connect to {address}: {source}")]
    Connect {
        address: SocketAddr,
        source: std::io::Error,
    },
    #[error("the feeder did not answer in time")]
    Timeout,
    /// The request went, and the session gave up on its answer.
    #[error("the feeder did not answer the request")]
    Unanswered,
    #[error("the feeder closed the connection")]
    Closed,
    #[error("the feeder stopped answering")]
    Silent,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Session(wellfed_client::api::Error),
    /// Refused before a byte went: the request holds a value with no wire
    /// form (a newer build's `unknown`), or does not fit one frame. The
    /// session is as it was.
    #[error("a message has no wire form or does not fit one frame")]
    Encode,
    #[error("that request id is the keepalive's")]
    ReservedId,
    #[error("the feeder does not know this key (not enrolled, or revoked)")]
    UnknownKey,
    #[error("the feeder serves protocol {0}; update this tool or the feeder")]
    Version(u16),
    #[error("every session slot is taken")]
    Full,
    /// The feeder runs a build from before API revisions, which takes only
    /// frozen ops: nothing was sent.
    #[error(
        "the feeder runs a build from before API revisions, which takes only an update; \
         nothing was sent: update the feeder"
    )]
    Legacy,
}

impl From<wellfed_client::api::Error> for Error {
    fn from(error: wellfed_client::api::Error) -> Self {
        use wellfed_client::api::Error as Session;
        match error {
            Session::Refused(OpenRefusal::Unknown) => Self::UnknownKey,
            Session::Refused(OpenRefusal::Version { supported }) => Self::Version(supported),
            Session::Refused(OpenRefusal::Full) => Self::Full,
            Session::Timeout => Self::Timeout,
            Session::Silent => Self::Silent,
            Session::ReservedId => Self::ReservedId,
            Session::Legacy => Self::Legacy,
            Session::TooLarge | Session::NoWireForm(_) => Self::Encode,
            other => Self::Session(other),
        }
    }
}

impl Error {
    /// For an error [`Connection::request`] returned: whether the session
    /// is as it was. Nothing of that request went either way; a request
    /// with no wire form, under the keepalive's id, or of a kind a
    /// pre-revision build does not take is that request's alone, while any
    /// other error is the session's end, and each request still
    /// outstanding on it may have run.
    pub const fn keeps_session(&self) -> bool {
        matches!(self, Self::Encode | Self::ReservedId | Self::Legacy)
    }
}

/// Why [`Connection::ask`] has no answer, by what it says of the request:
/// the one place a client learns whether a write may have run.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// Refused before a byte went: the request did not run.
    /// [`Error::keeps_session`] says whether the session is as it was.
    #[error(transparent)]
    Unsent(Error),
    /// The request was queued, and its answer never came: the session gave
    /// up on it, or ended. A write may have run, and is never sent again.
    #[error("{0}; the request may have run")]
    MayHaveRun(Error),
}

impl AskError {
    /// The session's error, for a caller to whom an unanswered read and an
    /// unsent one are alike.
    pub fn into_error(self) -> Error {
        match self {
            Self::Unsent(error) | Self::MayHaveRun(error) => error,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An address as an owner types it: an IP takes the API's port.
pub fn parse_address(address: &str) -> Result<SocketAddr> {
    if let Ok(ip) = address.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, PORT));
    }
    address
        .parse()
        .map_err(|_| Error::Address(address.to_owned()))
}

/// The feeder's address: `address` when given (an IP takes the API's port),
/// otherwise the feeder with `identity` found over mDNS within `wait`.
///
/// mDNS browsing blocks the calling thread for up to `wait`.
pub fn locate(identity: &[u8; 32], address: Option<&str>, wait: Duration) -> Result<SocketAddr> {
    if let Some(address) = address {
        return parse_address(address);
    }

    let browse = Browse::start()?;
    let want = Lower(identity).to_string();
    let deadline = std::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match browse.next(left) {
            Browsed::Found(feeder) if feeder.identity == want => return Ok(feeder.address),
            Browsed::Found(_) => {}
            Browsed::Quiet | Browsed::Stopped => {
                return Err(Error::NotFound(want[..16].to_owned()));
            }
        }
    }
}

/// A feeder announcing its device API over mDNS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announced {
    /// Its identity key as the TXT `id` gives it: lowercase hex.
    pub identity: String,
    /// Where its device API listens, on the first IPv4 address it gave.
    pub address: SocketAddr,
    /// The service instance name it announces.
    pub name: String,
}

/// What [`Browse::next`] heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Browsed {
    Found(Announced),
    /// Nothing within the wait.
    Quiet,
    /// The browse was stopped, or its daemon ended.
    Stopped,
}

/// Feeders announcing themselves over mDNS, until [`Self::stop`] or drop.
///
/// A feeder is heard again each time it re-announces, so a caller that
/// lists them keys them by identity.
pub struct Browse {
    daemon: mdns_sd::ServiceDaemon,
    receiver: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
}

impl Browse {
    pub fn start() -> Result<Self> {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        let receiver = daemon.browse(&format!("{SERVICE}.local."))?;
        Ok(Self { daemon, receiver })
    }

    /// The next feeder resolved within `wait`. Blocks the calling thread, so
    /// an async caller runs it on a blocking thread.
    pub fn next(&self, wait: Duration) -> Browsed {
        let deadline = std::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Browsed::Quiet;
            }
            let event = match self.receiver.recv_timeout(left) {
                Ok(event) => event,
                Err(mdns_sd::RecvTimeoutError::Timeout) => return Browsed::Quiet,
                Err(mdns_sd::RecvTimeoutError::Disconnected) => return Browsed::Stopped,
            };
            match event {
                mdns_sd::ServiceEvent::ServiceResolved(service) => {
                    if let Some(feeder) = announced(&service) {
                        return Browsed::Found(feeder);
                    }
                }
                mdns_sd::ServiceEvent::SearchStopped(_) => return Browsed::Stopped,
                _ => {}
            }
        }
    }

    /// Stop browsing: a blocked [`Self::next`] answers [`Browsed::Stopped`].
    pub fn stop(&self) {
        // Shutting down only stops the daemon's thread, and a second
        // shutdown fails harmlessly; nothing is left that matters.
        let _ = self.daemon.shutdown();
    }
}

impl Drop for Browse {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A resolved `_wellfed._tcp` service as a feeder: one with an identity
/// and an IPv4 address, since the device API listens on IPv4 only.
fn announced(service: &mdns_sd::ResolvedService) -> Option<Announced> {
    let identity = service.get_property_val_str("id")?;
    let ip = service.get_addresses_v4().into_iter().next()?;
    let fullname = service.get_fullname();
    let name = fullname
        .strip_suffix(service.ty_domain.as_str())
        .and_then(|name| name.strip_suffix('.'))
        .unwrap_or(fullname);
    Some(Announced {
        identity: identity.to_owned(),
        address: SocketAddr::new(IpAddr::V4(ip), service.get_port()),
        name: name.to_owned(),
    })
}

/// The clock the sans-IO session reads: tokio's, so a paused test clock
/// drives it too.
fn now() -> std::time::Instant {
    Instant::now().into_std()
}

async fn dial(address: SocketAddr) -> Result<TcpStream> {
    let stream = timeout(CONNECT, TcpStream::connect(address))
        .await
        .map_err(|_| Error::ConnectTimeout(address))?
        .map_err(|source| Error::Connect { address, source })?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// Bytes the socket has not yet taken. A write whose future was dropped
/// leaves its rest here, and it goes out before anything else, so frames
/// never interleave.
#[derive(Debug, Default)]
struct Outbox(Vec<u8>);

impl Outbox {
    /// Send what the outbox holds. Each `write` either takes bytes or
    /// leaves them, and they leave the outbox in the same poll, so a
    /// dropped future loses or repeats nothing.
    async fn flush(&mut self, stream: &mut TcpStream) -> Result<()> {
        while !self.0.is_empty() {
            let n = stream.write(&self.0).await?;
            if n == 0 {
                return Err(Error::Closed);
            }
            self.0.drain(..n);
        }
        Ok(())
    }
}

/// What [`Connection::receive`] yields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// An event, gap, sample or log batch the feeder pushed.
    Pushed(Pushed),
    /// The answer to the request under `id`, or `None` when none came in
    /// time or the session ended first: the request may have run.
    Answer {
        id: RequestId,
        answer: Option<Answer>,
    },
}

/// An open session.
pub struct Connection {
    stream: TcpStream,
    session: connection::Connection,
    outbox: Outbox,
    /// Pushed messages not yet taken by [`Self::next`].
    queued: VecDeque<Pushed>,
    /// The newest event or gap end handed to the caller, or the cursor the
    /// subscription resumed after: behind the session's while events wait
    /// in `queued`.
    cursor: Option<Cursor>,
    /// Answers, or their absence, not yet taken by [`Self::ask`] or
    /// [`Self::receive`].
    answers: VecDeque<(RequestId, Option<Answer>)>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("opened", &self.session.opened())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Open a session with the feeder `identity` at `address` as `keys`;
    /// `ephemeral` is fresh from the OS generator.
    pub async fn open(
        keys: &Keypair,
        identity: [u8; 32],
        address: SocketAddr,
        ephemeral: [u8; 32],
    ) -> Result<Self> {
        Self::open_with(keys, identity, address, ephemeral, Timing::DEFAULT).await
    }

    /// As [`Self::open`], on the session clocks `timing` gives rather than
    /// [`Timing::DEFAULT`]: a test that loses an answer on purpose waits
    /// out a short answer window instead of the feeder's real one.
    pub async fn open_with(
        keys: &Keypair,
        identity: [u8; 32],
        address: SocketAddr,
        ephemeral: [u8; 32],
        timing: Timing,
    ) -> Result<Self> {
        // The welcome's window starts once the connection is up, not
        // before a slow accept.
        let stream = dial(address).await?;
        let session =
            connection::Connection::with(keys, identity, ephemeral, Hello::CURRENT, timing, now())?;
        let mut connection = Self {
            stream,
            session,
            outbox: Outbox::default(),
            queued: VecDeque::new(),
            cursor: None,
            answers: VecDeque::new(),
        };
        while connection.session.opened().is_none() {
            connection.turn(None).await?;
        }
        Ok(connection)
    }

    /// Whether the device's kind serves `op`. Another kind's op is never
    /// sent: it is answered here as the device would answer it.
    fn serves(&self, op: &Op) -> bool {
        serves(self.kind(), op)
    }

    fn opened(&self) -> &Opened {
        self.session
            .opened()
            .expect("a Connection exists only once welcomed")
    }

    /// The feeder's build, from its welcome.
    pub fn build(&self) -> &str {
        &self.opened().build
    }

    /// The role the feeder gave this key.
    pub fn role(&self) -> ClientRole {
        self.opened().role
    }

    /// Which module of the device API the device serves.
    pub fn kind(&self) -> DeviceKind {
        self.opened().kind
    }

    /// The device's model.
    pub fn model(&self) -> Model {
        self.opened().model
    }

    /// The epoch the feeder's event numbers count in.
    pub fn epoch(&self) -> u32 {
        self.opened().epoch
    }

    /// Which side is the older build ([`wellfed_model::Skew`]): every op
    /// goes either way, and this says what to update when one is missing.
    pub fn skew(&self) -> wellfed_model::Skew {
        self.opened().skew()
    }

    /// The newest event or gap end [`Self::next`] or [`Self::receive`]
    /// handed out since [`Self::subscribe`], or the cursor it resumed
    /// after. One read may decode several events at once; those not yet
    /// handed out are still ahead of it, so a caller that keeps it and
    /// ends the session resumes before them.
    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    /// The next pushed message, moving [`Self::cursor`] past it.
    fn hand_on(&mut self) -> Option<Pushed> {
        let message = self.queued.pop_front()?;
        if let Some(cursor) = Cursor::after(&message) {
            self.cursor = Some(cursor);
        }
        Some(message)
    }

    /// Change how long a silent feeder is given before the session ends with
    /// [`Error::Silent`]; [`SILENCE`] is the usual limit. The caller keeps a
    /// shorter limit fed with its own requests.
    pub fn set_silence(&mut self, silence: Duration) {
        self.session.set_silence(silence);
    }

    /// Messages that arrived while [`Self::ask`] waited, not yet taken by
    /// [`Self::next`].
    pub fn queued(&self) -> usize {
        self.queued.len()
    }

    /// One turn of the loop: write what the session has, then read or
    /// wake for its clock, until `until` at the latest. Cancel-safe.
    async fn turn(&mut self, until: Option<Instant>) -> Result<()> {
        let result = self.step(until).await;

        // What the session decoded before an error ended it still counts:
        // an answer read in the same read as a bad frame is that request's
        // answer, not its absence.
        while let Some(event) = self.session.poll_event() {
            match event {
                Event::Pushed(message) => self.queued.push_back(message),
                Event::Answer { id, answer } => self.answers.push_back((id, Some(answer))),
                Event::Unanswered { id } => self.answers.push_back((id, None)),
            }
        }
        result
    }

    async fn step(&mut self, until: Option<Instant>) -> Result<()> {
        while let Some(bytes) = self.session.poll_transmit() {
            self.outbox.0.extend(bytes);
        }
        let due = self.session.poll_timeout().map(Instant::from_std);
        let wake = match (due, until) {
            (Some(due), Some(until)) => due.min(until),
            (due, until) => due.or(until).unwrap_or_else(|| Instant::now() + SILENCE),
        };

        // The write is bounded by the same clock as the read: a feeder that
        // stopped reading fills the socket's buffer, and the silence limit
        // must still end the session rather than the kernel, minutes on.
        // The outbox keeps what a cut-short flush did not write, and a
        // write the socket takes at once goes even past the clock.
        let Ok(flushed) = timeout_at(wake, self.outbox.flush(&mut self.stream)).await else {
            self.session.handle_timeout(now())?;
            return Ok(());
        };
        flushed?;

        // A clock already due acts before any read: a session silent past
        // its limit is gone even when a close is waiting to be read.
        let mut buffer = [0u8; 2048];
        let read = if wake <= Instant::now() {
            Err(())
        } else {
            timeout_at(wake, self.stream.read(&mut buffer))
                .await
                .map_err(drop)
        };
        match read {
            Ok(read) => {
                let n = read?;
                if n == 0 {
                    return Err(Error::Closed);
                }
                self.session.handle_input(now(), &buffer[..n])?;
            }
            Err(_) => self.session.handle_timeout(now())?,
        }
        Ok(())
    }

    /// Ask for the history after `from`, the newest event the caller holds
    /// (`None`: the oldest kept), then each new one, and the live samples
    /// if `samples`. A cursor from an epoch the feeder no longer has starts
    /// from the oldest kept: [`Resume::Restarted`].
    pub async fn subscribe(&mut self, from: Option<Cursor>, samples: bool) -> Result<Resume> {
        let resume = self.session.subscribe(now(), from, samples)?;
        self.cursor = self.session.cursor();
        while let Some(bytes) = self.session.poll_transmit() {
            self.outbox.0.extend(bytes);
        }
        self.outbox.flush(&mut self.stream).await?;
        Ok(resume)
    }

    /// Queue one request under `id` without waiting for its answer; it is
    /// written by the next [`Self::receive`], which later yields its answer
    /// or its absence. Several may be outstanding. `id` may not be
    /// [`KEEPALIVE_ID`]. Answers go to [`Self::receive`] only: [`Self::ask`]
    /// and [`Self::next`] drop those of other requests.
    pub fn request(&mut self, id: RequestId, op: &Op) -> Result<()> {
        if !self.serves(op) {
            self.answers.push_back((id, Some(UNAVAILABLE)));
            return Ok(());
        }
        self.session.request(now(), id, op)?;
        while let Some(bytes) = self.session.poll_transmit() {
            self.outbox.0.extend(bytes);
        }
        Ok(())
    }

    /// The next answer or pushed message within `wait`, writing what is
    /// queued and keeping the session alive meanwhile. Cancel-safe. When
    /// the session ends, the error comes back and the requests still
    /// outstanding get no answer here: each may have run.
    pub async fn receive(&mut self, wait: Duration) -> Result<Received> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some((id, answer)) = self.answers.pop_front() {
                return Ok(Received::Answer { id, answer });
            }
            if let Some(message) = self.hand_on() {
                return Ok(Received::Pushed(message));
            }
            if deadline <= Instant::now() {
                return Err(Error::Timeout);
            }
            self.turn(Some(deadline)).await?;
        }
    }

    /// The next pushed message within `wait`, keeping the session alive
    /// meanwhile.
    pub async fn next(&mut self, wait: Duration) -> Result<Pushed> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(message) = self.hand_on() {
                return Ok(message);
            }
            // Answers belong to `ask` and `receive`; one given up on is dropped.
            self.answers.clear();
            if deadline <= Instant::now() {
                return Err(Error::Timeout);
            }
            self.turn(Some(deadline)).await?;
        }
    }

    /// One request under `id` and its answer. Other messages that arrive
    /// meanwhile stay queued for [`Self::next`]. `id` may not be
    /// [`KEEPALIVE_ID`]. Once the request is queued, a session that ends
    /// or an answer that never comes says nothing of whether it ran: the
    /// error says which side of that line it fell.
    pub async fn ask(&mut self, id: RequestId, op: &Op) -> Result<Answer, AskError> {
        if !self.serves(op) {
            return Ok(UNAVAILABLE);
        }
        self.session
            .request(now(), id, op)
            .map_err(|error| AskError::Unsent(error.into()))?;

        loop {
            if let Some(answer) = self.answer_to(id) {
                return answer.ok_or(AskError::MayHaveRun(Error::Unanswered));
            }
            if let Err(error) = self.turn(None).await {
                // The read that ended the session may have carried the
                // answer first.
                return match self.answer_to(id) {
                    Some(Some(answer)) => Ok(answer),
                    _ => Err(AskError::MayHaveRun(error)),
                };
            }
        }
    }

    /// The answer, or its absence, to the request under `id` once it is
    /// in; the answers to other requests before it are dropped.
    fn answer_to(&mut self, id: RequestId) -> Option<Option<Answer>> {
        while let Some((answered, answer)) = self.answers.pop_front() {
            if answered == id {
                return Some(answer);
            }
        }
        None
    }
}

/// The feeder's recovery window over the device API, as an owner key that
/// proves the setup code.
pub struct Recovery {
    stream: TcpStream,
    window: Window,
    outbox: Outbox,
}

impl std::fmt::Debug for Recovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recovery").finish_non_exhaustive()
    }
}

impl Recovery {
    /// Open the recovery session at `address` with `session`'s setup
    /// exchange.
    pub async fn open(
        address: SocketAddr,
        session: wellfed_client::setup::Session,
    ) -> Result<Self> {
        Ok(Self {
            stream: dial(address).await?,
            window: Window::new(session)?,
            outbox: Outbox::default(),
        })
    }

    /// The feeder's next reply within `wait`; the session answers the
    /// proof's steps itself meanwhile. A closed connection means the
    /// window is not open.
    pub async fn reply(&mut self, wait: Duration) -> Result<Reply> {
        let deadline = Instant::now() + wait;
        loop {
            while let Some(bytes) = self.window.poll_transmit() {
                self.outbox.0.extend(bytes);
            }
            self.outbox.flush(&mut self.stream).await?;
            if let Some(reply) = self.window.poll_reply() {
                return Ok(reply);
            }

            let mut buffer = [0u8; 1024];
            let n = timeout_at(deadline, self.stream.read(&mut buffer))
                .await
                .map_err(|_| Error::Timeout)??;
            if n == 0 {
                return Err(Error::Closed);
            }
            self.window.handle_input(&buffer[..n])?;
        }
    }

    /// The setup session: its code, and the feeder's key once proven.
    pub fn session(&self) -> &wellfed_client::setup::Session {
        self.window.session()
    }
}

/// Redeem `invite` at `address` as `keys`, listed as `label`: the role the
/// feeder enrolled the key with, or its refusal.
pub async fn join(
    keys: &Keypair,
    invite: Invite,
    label: &str,
    address: SocketAddr,
    ephemeral: [u8; 32],
) -> Result<Joined> {
    let (mut join, hello) = Join::connect(keys, invite, label, ephemeral)?;
    let mut stream = dial(address).await?;
    write_within(&mut stream, &hello).await?;

    let mut buffer = [0u8; 2048];
    loop {
        let n = timeout(ANSWER, stream.read(&mut buffer))
            .await
            .map_err(|_| Error::Timeout)??;
        if n == 0 {
            // The feeder drops an unknown or expired invite unanswered.
            return Err(Error::Closed);
        }
        match join.receive(&buffer[..n])? {
            None => {}
            Some(JoinStep::Send(bytes)) => write_within(&mut stream, &bytes).await?,
            Some(JoinStep::Done(joined)) => return Ok(joined),
        }
    }
}

/// Write `bytes` within [`ANSWER`]: a feeder that stopped reading would
/// otherwise hold the write until the kernel gives up.
async fn write_within(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    timeout(ANSWER, stream.write_all(bytes))
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(())
}

/// Whether a device of `kind` serves `op`: the shared core, or its own
/// module's.
fn serves(kind: DeviceKind, op: &Op) -> bool {
    op.kind().is_none_or(|of| of == kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wellfed_model::StatsSection;

    /// A request with no wire form is refused before a byte went, as one
    /// too large is: the caller answers it unsendable and keeps the
    /// session, so the writes in flight on it stay known.
    #[test]
    fn a_request_with_no_wire_form_leaves_the_session_up() {
        let no_wire_form =
            wellfed_client::api::Error::NoWireForm(wellfed_model::convert::NoWireForm);
        assert!(matches!(Error::from(no_wire_form), Error::Encode));
        assert!(matches!(
            Error::from(wellfed_client::api::Error::TooLarge),
            Error::Encode
        ));
    }

    /// Another kind's op, a feeder-only statistics section among them, is
    /// answered here and never sent; the shared core goes to any kind.
    #[test]
    fn another_kind_s_op_is_refused_locally() {
        let feed = Op::Feed { portions: 1 };
        let food = Op::Stats {
            section: StatsSection::Food,
        };
        let counters = Op::Stats {
            section: StatsSection::Counters,
        };
        for op in [&feed, &food, &Op::GetPlans] {
            assert!(serves(DeviceKind::Feeder, op), "{op:?}");
            assert!(!serves(DeviceKind::Fountain, op), "{op:?}");
        }
        for op in [&Op::Status, &counters, &Op::Reboot] {
            assert!(serves(DeviceKind::Fountain, op), "{op:?}");
        }
        assert_eq!(UNAVAILABLE, Answer::Refused(Refusal::Unavailable));
    }

    /// A fountain's side of one session: the handshake, welcoming the
    /// client as a PLWF106, then a count of every frame the client sends
    /// after it, until the client hangs up.
    fn fountain(listener: &std::net::TcpListener, device: &Keypair) -> usize {
        use std::io::{Read, Write};
        use wellfed_proto::api::{
            FRAME_MAX, Hello, Open, PROLOGUE, PROTOCOL_MAJOR, Role as WireRole, Welcome,
        };
        use wellfed_proto::device::Model;
        use wellfed_session::frame::{self, PREFIX};
        use wellfed_session::noise::{Handshake, Pattern, Role};

        fn read_frame(stream: &mut std::net::TcpStream) -> Option<Vec<u8>> {
            let mut prefix = [0; PREFIX];
            stream.read_exact(&mut prefix).ok()?;
            let len = frame::parse_prefix(prefix, FRAME_MAX).unwrap();
            let mut message = vec![0; len];
            stream.read_exact(&mut message).ok()?;
            Some(message)
        }

        let (mut stream, _) = listener.accept().unwrap();
        let open: Open = wellfed_proto::decode(&read_frame(&mut stream).unwrap()).unwrap();
        assert_eq!(open, Open::Session);
        let mut handshake = Handshake::new(
            Pattern::Ik,
            Role::Responder,
            PROLOGUE,
            device,
            None,
            [7; 32],
        )
        .unwrap();
        let mut plain = [0; FRAME_MAX];
        let len = handshake
            .read_message(&read_frame(&mut stream).unwrap(), &mut plain)
            .unwrap();
        let hello: Hello = wellfed_proto::decode(&plain[..len]).unwrap();
        assert_eq!(hello.major, PROTOCOL_MAJOR);

        let welcome = Welcome::Open {
            major: PROTOCOL_MAJOR,
            model: Model::Plwf106,
            build: "fountain",
            role: WireRole::Owner,
            epoch: 1,
            schema: wellfed_proto::api::REVISION,
        };
        let len = wellfed_proto::encode(&welcome, &mut plain).unwrap();
        let mut out = [0; FRAME_MAX];
        let written = handshake.write_message(&plain[..len], &mut out).unwrap();
        stream
            .write_all(&frame::prefix(written, FRAME_MAX).unwrap())
            .unwrap();
        stream.write_all(&out[..written]).unwrap();

        let mut after = 0;
        while read_frame(&mut stream).is_some() {
            after += 1;
        }
        after
    }

    /// A feeder op asked of a fountain over a real session, through both
    /// `request` and `ask`: answered `Unavailable` here, and not one byte
    /// of it queued or sent.
    #[test]
    fn another_kind_s_op_never_leaves_the_connection() {
        let device = Keypair::from_secret([0x10; 32]);
        let identity = *device.public();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let served = std::thread::spawn(move || fountain(&listener, &device));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let keys = Keypair::from_secret([0x20; 32]);
            let mut connection = Connection::open(&keys, identity, address, [9; 32])
                .await
                .unwrap();
            assert_eq!(connection.kind(), DeviceKind::Fountain);

            connection
                .request([1; 16], &Op::Feed { portions: 1 })
                .unwrap();
            let asked = connection.ask([2; 16], &Op::GetPlans).await.unwrap();
            assert_eq!(asked, UNAVAILABLE);
            let requested = connection.receive(Duration::ZERO).await.unwrap();
            assert_eq!(
                requested,
                Received::Answer {
                    id: [1; 16],
                    answer: Some(UNAVAILABLE),
                }
            );

            assert!(
                connection.outbox.0.is_empty(),
                "nothing waits to be written"
            );
            assert!(
                connection.session.poll_transmit().is_none(),
                "the session sealed nothing"
            );
        });

        // The connection is dropped: the device saw the handshake and no
        // frame after it.
        assert_eq!(served.join().unwrap(), 0);
    }
}
