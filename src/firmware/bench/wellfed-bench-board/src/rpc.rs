//! The control interface: postcard-rpc requests on the vendor bulk interface.
//!
//! A request that switches an output takes the outputs now or is refused
//! [`Refusal::Held`]: a RECOVER sequence holds them for seconds, and a
//! request the host saw refused must never run later. A status request
//! reads the last state and never waits.

use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Endpoint, In, Out};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use postcard_rpc::define_dispatch;
use postcard_rpc::header::VarHeader;
use postcard_rpc::server::{Dispatch, Server, SpawnContext};
use static_cell::StaticCell;
use wellfed_bench_icd::{
    Bootsel, BootselReply, ENDPOINT_LIST, GetStatus, Refusal, Set, SetChannel, SetReply, Status,
    TOPICS_IN_LIST, TOPICS_OUT_LIST,
};

use crate::wire::{NoSpawn, Rx, Tx, TxInner};
use crate::{outputs, telemetry};

/// Largest frame either way. Every message here is a few dozen bytes; the
/// standard schema report goes out a type at a time.
const FRAME_MAX: usize = 256;

#[expect(unreachable_pub, reason = "define_dispatch! declares a pub Dispatcher whose interface names this type")]
pub struct Context;

impl SpawnContext for Context {
    type SpawnCtxt = ();

    fn spawn_ctxt(&mut self) {}
}

define_dispatch! {
    app: Dispatcher;
    spawn_fn: spawn_fn;
    tx_impl: Tx;
    spawn_impl: NoSpawn;
    context: Context;

    endpoints: {
        list: ENDPOINT_LIST;

        | EndpointTy | kind     | handler    |
        | ---------- | ----     | -------    |
        | SetChannel | blocking | set        |
        | GetStatus  | blocking | get_status |
        | Bootsel    | blocking | bootsel    |
    };
    topics_in: {
        list: TOPICS_IN_LIST;

        | TopicTy | kind | handler |
        | ------- | ---- | ------- |
    };
    topics_out: {
        list: TOPICS_OUT_LIST;
    };
}

fn set(_: &mut Context, _: VarHeader, request: Set) -> SetReply {
    let mut outputs = outputs::try_lock().ok_or(Refusal::Held)?;
    outputs.set(request.channel, request.on);
    Ok(outputs.state())
}

fn get_status(_: &mut Context, _: VarHeader, (): ()) -> Status {
    Status {
        channels: outputs::snapshot(),
        cuts: outputs::cuts(),
        telemetry: telemetry::latest(),
    }
}

/// Cuts the feeder's power now, under the same refusal as a set, and leaves
/// the reset to [`reboot`] so the reply goes out first.
fn bootsel(_: &mut Context, _: VarHeader, (): ()) -> BootselReply {
    let mut outputs = outputs::try_lock().ok_or(Refusal::Held)?;
    outputs.cut_power();
    BOOTSEL.signal(());
    Ok(())
}

static BOOTSEL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
pub(crate) async fn reboot() -> ! {
    BOOTSEL.wait().await;
    // Let the host read the reply before the device drops off the bus.
    Timer::after(Duration::from_millis(100)).await;
    embassy_rp::rom_data::reset_to_usb_boot(0, 0);
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
pub(crate) async fn run(ep_in: Endpoint<'static, USB, In>, ep_out: Endpoint<'static, USB, Out>) -> ! {
    static TX: StaticCell<Option<Mutex<CriticalSectionRawMutex, TxInner>>> = StaticCell::new();
    static TX_BUF: StaticCell<[u8; FRAME_MAX]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; FRAME_MAX]> = StaticCell::new();

    let tx = Tx::new(TX.init(None), ep_in, TX_BUF.init([0; FRAME_MAX]));
    let dispatcher = Dispatcher::new(Context, NoSpawn);
    let kkind = dispatcher.min_key_len();
    let mut server = Server::new(tx, Rx { ep_out }, RX_BUF.init([0; FRAME_MAX]).as_mut_slice(), dispatcher, kkind);

    loop {
        // An unplug or bus reset ends a run; the next waits for the host.
        let _ = server.run().await;
    }
}
