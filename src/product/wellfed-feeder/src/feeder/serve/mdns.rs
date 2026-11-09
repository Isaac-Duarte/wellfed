//! The mDNS socket and the responder that advertises `_wellfed._tcp`
//! (`wellfed_core::mdns`). The responder exists once the identity is
//! loaded; its socket opens with the listener ([`Serve::open_sockets`]) and
//! shares its retry.

use wellfed_core::mdns::{self, Destination};
use wellfed_device::io::Datagram;

use super::super::platform::Platform;
use super::{RETRY_MS, Serve};

/// mDNS datagrams read per pass, so a storm of queries costs a bounded turn.
const DATAGRAMS: usize = 4;

impl<S: Copy> Serve<S> {
    pub(super) fn mdns<P: Platform<Socket = S>>(&mut self, platform: &mut P, now_ms: u32) {
        let (Some(socket), Some(responder)) = (self.mdns_socket, self.responder.as_mut()) else {
            return;
        };
        if let Some(len) = responder.announce(now_ms, &mut self.reply)
            && !platform.mdns_send(socket, &self.reply[..len], mdns::GROUP, mdns::PORT)
        {
            self.close_mdns(platform, now_ms);
            return;
        }
        for _ in 0..DATAGRAMS {
            match platform.mdns_receive(socket, &mut self.packet) {
                Datagram::None => return,
                Datagram::Error => return self.close_mdns(platform, now_ms),
                Datagram::Got { len, ip, port } => {
                    self.queries = self.queries.wrapping_add(1);
                    let Some(reply) = responder.query(&self.packet[..len], port, &mut self.reply)
                    else {
                        continue;
                    };
                    let answer = &self.reply[..reply.len];
                    let (to, to_port) = match reply.to {
                        Destination::Multicast => (mdns::GROUP, mdns::PORT),
                        Destination::Unicast => (ip, port),
                    };
                    let sent = platform.mdns_send(socket, answer, to, to_port);
                    // A lost answer is asked again; it is only counted.
                    if sent {
                        self.answers = self.answers.wrapping_add(1);
                    } else {
                        self.failed = self.failed.wrapping_add(1);
                    }
                }
            }
        }
    }

    fn close_mdns<P: Platform<Socket = S>>(&mut self, platform: &mut P, now_ms: u32) {
        if let Some(socket) = self.mdns_socket.take() {
            platform.close(socket);
        }
        self.retry_at = now_ms.wrapping_add(RETRY_MS);
    }
}
