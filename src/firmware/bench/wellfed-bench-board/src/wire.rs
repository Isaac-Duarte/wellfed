//! postcard-rpc's wire over the board's vendor bulk endpoints.
//!
//! postcard-rpc ships this transport only for embassy-usb 0.5, whose
//! embassy-sync and embassy-executor versions differ from the board's. This
//! is that transport (postcard-rpc 0.12.1, `server/impls/embassy_usb_v0_5.rs`,
//! MIT OR Apache-2.0, by James Munns) narrowed to the RP2040's endpoints.
//!
//! A postcard-rpc frame is one USB transfer: it ends at the first packet
//! shorter than [`PACKET`], so a frame that fills its last packet is
//! followed by an empty one.

use core::convert::Infallible;
use core::fmt::{Arguments, Write};

use embassy_futures::select::{Either, select};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Endpoint as RpEndpoint, In, Out};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embassy_usb::driver::{Endpoint, EndpointError, EndpointIn, EndpointOut};
use postcard_rpc::Topic;
use postcard_rpc::header::{VarHeader, VarKey, VarKeyKind, VarSeq};
use postcard_rpc::server::{WireRx, WireRxErrorKind, WireSpawn, WireTx, WireTxErrorKind};
use postcard_rpc::standard_icd::LoggingTopic;
use serde::Serialize;

use crate::usb::PACKET;

/// Per USB packet: a frame that is not taken within this fails, so an
/// unplugged host cannot stall the dispatch loop.
const TIMEOUT_MS_PER_PACKET: u64 = 2;

/// Longest log line; the board sends none today.
const LOG_MAX: usize = 64;

pub(crate) struct TxInner {
    ep_in: RpEndpoint<'static, USB, In>,
    buf: &'static mut [u8],
    log_seq: u16,
    /// A send timed out mid-frame; the next starts with an empty packet so
    /// the host drops the half frame rather than joining it to the next.
    pending: bool,
}

#[derive(Clone, Copy)]
#[expect(unreachable_pub, reason = "define_dispatch! declares a pub Dispatcher whose interface names this type")]
pub struct Tx {
    inner: &'static Mutex<CriticalSectionRawMutex, TxInner>,
}

impl Tx {
    pub(crate) fn new(
        inner: &'static mut Option<Mutex<CriticalSectionRawMutex, TxInner>>,
        ep_in: RpEndpoint<'static, USB, In>,
        buf: &'static mut [u8],
    ) -> Self {
        let inner = inner.insert(Mutex::new(TxInner {
            ep_in,
            buf,
            log_seq: 0,
            pending: false,
        }));
        Self { inner }
    }
}

impl WireTx for Tx {
    type Error = WireTxErrorKind;

    async fn wait_connection(&self) {
        self.inner.lock().await.ep_in.wait_enabled().await;
    }

    async fn send<T: Serialize + ?Sized>(&self, hdr: VarHeader, msg: &T) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        let TxInner {
            ep_in, buf, pending, ..
        } = &mut *inner;

        let used = encode(buf, hdr, msg)?;
        send_all(ep_in, &buf[..used], pending).await
    }

    async fn send_raw(&self, frame: &[u8]) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        let TxInner { ep_in, pending, .. } = &mut *inner;
        send_all(ep_in, frame, pending).await
    }

    async fn send_log_str(&self, kkind: VarKeyKind, s: &str) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        let TxInner {
            ep_in,
            buf,
            log_seq,
            pending,
        } = &mut *inner;

        let key = match kkind {
            VarKeyKind::Key1 => VarKey::Key1(LoggingTopic::TOPIC_KEY1),
            VarKeyKind::Key2 => VarKey::Key2(LoggingTopic::TOPIC_KEY2),
            VarKeyKind::Key4 => VarKey::Key4(LoggingTopic::TOPIC_KEY4),
            VarKeyKind::Key8 => VarKey::Key8(LoggingTopic::TOPIC_KEY),
        };
        let hdr = VarHeader {
            key,
            seq_no: VarSeq::Seq2(*log_seq),
        };
        *log_seq = log_seq.wrapping_add(1);

        let used = encode(buf, hdr, s)?;
        send_all(ep_in, &buf[..used], pending).await
    }

    async fn send_log_fmt<'a>(&self, kkind: VarKeyKind, args: Arguments<'a>) -> Result<(), Self::Error> {
        let mut line = heapless::String::<LOG_MAX>::new();
        // A line too long for LOG_MAX is sent cut short.
        let _ = line.write_fmt(args);
        self.send_log_str(kkind, &line).await
    }
}

fn encode<T: Serialize + ?Sized>(buf: &mut [u8], hdr: VarHeader, msg: &T) -> Result<usize, WireTxErrorKind> {
    let (hdr_used, rest) = hdr.write_to_slice(buf).ok_or(WireTxErrorKind::Other)?;
    let hdr_len = hdr_used.len();
    let body_len = postcard::to_slice(msg, rest)
        .map_err(|_| WireTxErrorKind::Other)?
        .len();
    Ok(hdr_len + body_len)
}

async fn send_all(
    ep_in: &mut RpEndpoint<'static, USB, In>,
    frame: &[u8],
    pending: &mut bool,
) -> Result<(), WireTxErrorKind> {
    if frame.is_empty() {
        return Ok(());
    }
    let packet = usize::from(PACKET);
    let packets = u64::try_from(frame.len().div_ceil(packet)).unwrap_or(u64::MAX);

    let send = async {
        if *pending && ep_in.write(&[]).await.is_err() {
            return Err(WireTxErrorKind::ConnectionClosed);
        }
        *pending = true;

        for chunk in frame.chunks(packet) {
            if ep_in.write(chunk).await.is_err() {
                return Err(WireTxErrorKind::ConnectionClosed);
            }
        }
        if frame.len().is_multiple_of(packet) && ep_in.write(&[]).await.is_err() {
            return Err(WireTxErrorKind::ConnectionClosed);
        }

        *pending = false;
        Ok(())
    };

    let timeout = Timer::after_millis(packets.saturating_mul(TIMEOUT_MS_PER_PACKET));
    match select(send, timeout).await {
        Either::First(result) => result,
        Either::Second(()) => Err(WireTxErrorKind::Timeout),
    }
}

pub(crate) struct Rx {
    pub(crate) ep_out: RpEndpoint<'static, USB, Out>,
}

impl WireRx for Rx {
    type Error = WireRxErrorKind;

    async fn wait_connection(&mut self) {
        self.ep_out.wait_enabled().await;
    }

    async fn receive<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a mut [u8], Self::Error> {
        let packet = usize::from(PACKET);
        let mut used = 0;
        while used < buf.len() {
            let n = match self.ep_out.read(&mut buf[used..]).await {
                Ok(n) => n,
                Err(EndpointError::BufferOverflow) => return Err(WireRxErrorKind::ReceivedMessageTooLarge),
                Err(EndpointError::Disabled) => return Err(WireRxErrorKind::ConnectionClosed),
            };
            used += n;
            if n != packet {
                return Ok(&mut buf[..used]);
            }
        }

        // Out of room: drain the rest of this frame so the next one starts
        // clean, then refuse it.
        let mut scratch = [0u8; PACKET as usize];
        loop {
            match self.ep_out.read(&mut scratch).await {
                Ok(n) if n == packet => {}
                Ok(_) | Err(EndpointError::BufferOverflow) => {
                    return Err(WireRxErrorKind::ReceivedMessageTooLarge);
                }
                Err(EndpointError::Disabled) => return Err(WireRxErrorKind::ConnectionClosed),
            }
        }
    }
}

/// Every handler runs inline in the dispatch loop, so nothing is spawned.
#[derive(Clone)]
#[expect(unreachable_pub, reason = "define_dispatch! declares a pub Dispatcher whose interface names this type")]
pub struct NoSpawn;

impl WireSpawn for NoSpawn {
    type Error = Infallible;
    type Info = ();

    fn info(&self) -> &() {
        &()
    }
}
