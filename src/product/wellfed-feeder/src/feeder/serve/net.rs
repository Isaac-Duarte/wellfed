//! The API's TCP edge: the listener on [`PORT`] and a socket per session
//! slot, every call non-blocking so the loop never waits on a client. The
//! server owns the bytes; these move them between its buffers and the
//! platform's sockets, and close a slot's socket when either side ends it.

use wellfed_device::io::{Accepted, Received};
use wellfed_device_api::SESSIONS;
use wellfed_proto::api::PORT;

use super::super::platform::Platform;
use super::{RETRY_MS, Record, Serve};

impl<S: Copy> Serve<S> {
    pub(super) fn open_sockets<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        now_ms: u32,
        record: &mut impl FnMut(Record),
    ) {
        if !wellfed_core::deadline_passed(now_ms, self.retry_at) {
            return;
        }
        if self.listener.is_none() {
            self.listener = platform.listen(PORT);
            record(Record::Listen {
                mdns: false,
                open: self.listener.is_some(),
                heap: platform.heap().0,
            });
        }
        if self.mdns_socket.is_none() && self.responder.is_some() {
            self.mdns_socket = platform.mdns_open();
            record(Record::Listen {
                mdns: true,
                open: self.mdns_socket.is_some(),
                heap: platform.heap().0,
            });
        }
        if self.listener.is_none() || self.mdns_socket.is_none() {
            self.retry_at = now_ms.wrapping_add(RETRY_MS);
        }
    }

    pub(super) fn accept<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        now_ms: u32,
        record: &mut impl FnMut(Record),
    ) {
        let Some(listener) = self.listener else {
            return;
        };
        match platform.accept(listener) {
            Accepted::None => {}
            Accepted::Error => {
                platform.close(listener);
                self.listener = None;
                self.retry_at = now_ms.wrapping_add(RETRY_MS);
                record(Record::Listen {
                    mdns: false,
                    open: false,
                    heap: platform.heap().0,
                });
            }
            Accepted::Socket(socket) => {
                match self.server.accept(now_ms, |r| record(Record::Server(r))) {
                    // A slot still holding a socket was taken from the
                    // session idle longest: its connection closes now.
                    Some(slot) => {
                        if let Some(evicted) = self.sockets[slot].replace(socket) {
                            platform.close(evicted);
                        }
                    }
                    None => platform.close(socket),
                }
            }
        }
    }

    pub(super) fn receive<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        record: &mut impl FnMut(Record),
    ) {
        for slot in 0..SESSIONS {
            let Some(socket) = self.sockets[slot] else {
                continue;
            };
            let room = self.server.rx(slot);
            if room.is_empty() {
                continue;
            }
            match platform.receive(socket, room) {
                Received::Empty => {}
                Received::Data(n) => self.server.received(slot, n),
                Received::Closed => self.drop_slot(platform, slot, record),
            }
        }
    }

    pub(super) fn send<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        record: &mut impl FnMut(Record),
    ) {
        for slot in 0..SESSIONS {
            let Some(socket) = self.sockets[slot] else {
                continue;
            };
            let pending = self.server.tx(slot);
            if !pending.is_empty() {
                match platform.send(socket, pending) {
                    Some(n) => self.server.sent(slot, n),
                    None => {
                        self.drop_slot(platform, slot, record);
                        continue;
                    }
                }
            }
            if self.server.closing(slot) {
                self.drop_slot(platform, slot, record);
            }
        }
    }

    fn drop_slot<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        slot: usize,
        record: &mut impl FnMut(Record),
    ) {
        if let Some(socket) = self.sockets[slot].take() {
            platform.close(socket);
        }
        self.server.closed(slot, |r| record(Record::Server(r)));
    }
}
