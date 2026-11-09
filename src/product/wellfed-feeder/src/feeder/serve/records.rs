//! The device API's log records (`PLAPI1`): the server's own, wrapped
//! whole, and the sockets', mDNS's, the clock's and the minute's that the
//! pass adds. No record carries a key, a secret or a credential.

use core::fmt;

use wellfed_device_api as server;
use wellfed_proto::api::PORT;

use super::super::owner;
use super::super::time;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    Server(server::Record),
    /// The listener (or the mDNS socket) opened, or failed and will retry.
    Listen {
        mdns: bool,
        open: bool,
        /// The heap free just after, so each socket's cost shows.
        heap: u32,
    },
    /// The address mDNS advertises.
    Address {
        ip: u32,
    },
    Time(time::Record),
    /// Enrollments and revocations the settings writer recorded that the
    /// alert rules never heard of, its outbox being full: in the history,
    /// their alerts missed until the next boot's replay.
    Unheard {
        count: u32,
    },
    /// Once a minute: the heap, and the mDNS datagrams seen and answered.
    Stats {
        free: u32,
        min: u32,
        queries: u32,
        answers: u32,
        failed: u32,
    },
}

/// One record's body (`PLAPI1`).
pub fn describe(record: Record, f: &mut impl fmt::Write) -> fmt::Result {
    match record {
        Record::Server(record) => server_record(record, f),
        Record::Listen { mdns, open, heap } => write!(
            f,
            "listen socket={} open={} heap_free={heap}",
            if mdns { "mdns" } else { "api" },
            u8::from(open)
        ),
        Record::Address { ip } => {
            let [a, b, c, d] = ip.to_be_bytes();
            write!(f, "mdns address={a}.{b}.{c}.{d} port={PORT}")
        }
        Record::Time(record) => time::describe(record, f),
        Record::Unheard { count } => write!(f, "alerts unheard={count}"),
        Record::Stats {
            free,
            min,
            queries,
            answers,
            failed,
        } => write!(
            f,
            "stats heap_free={free} heap_min={min} mdns_in={queries} mdns_out={answers} mdns_failed={failed}"
        ),
    }
}

fn server_record(record: server::Record, f: &mut impl fmt::Write) -> fmt::Result {
    use server::Record as R;
    match record {
        R::Accepted { slot } => write!(f, "session slot={slot} state=accepted"),
        R::Opened {
            slot,
            role,
            client,
            handshake_ms,
        } => write!(
            f,
            "session slot={slot} state=open role={role} client={} handshake_ms={handshake_ms}",
            wellfed_session::hex::Lower(&client)
        ),
        R::Request { slot, op, result } => {
            write!(f, "request slot={slot} op={op} result={result}")
        }
        R::Retried { slot } => write!(f, "request slot={slot} retried=1"),
        R::Ended { slot, reason } => write!(f, "session slot={slot} state=ended reason={reason}"),
        R::Pairing(event) => owner::pair(event, f),
        R::UpdateKey { slot, change, key } => write!(
            f,
            "update-key slot={slot} change={change} id={}",
            wellfed_session::hex::Lower(&key)
        ),
    }
}
