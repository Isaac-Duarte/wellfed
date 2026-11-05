//! [`Sockets`], a simulated device's network on the host: the device API's
//! connections, its mDNS socket and its NTP exchange, answered as the
//! firmware's lwIP driver answers its loop (`wellfed_device::io`), every
//! call non-blocking.
//!
//! The listener belongs to the serve loop, which awaits its connections and
//! hands each one over ([`Sockets::accepted`]); the loop's driver then
//! takes it as it takes one from lwIP. While the radio is off no
//! connection is taken and those open are dropped, as the network leaving
//! does.

use std::collections::VecDeque;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use tokio::net::{TcpStream, UdpSocket};
use wellfed_device::io::{Accepted, Datagram, Received};

/// A socket the loop's driver holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Socket {
    /// The serve loop's listener.
    Listener,
    /// The mDNS socket, which drops everything when the simulator does not
    /// advertise.
    Mdns,
    /// The NTP exchange's socket.
    Ntp,
    /// One accepted connection.
    Stream(u32),
}

/// The device's sockets on the host.
#[derive(Debug)]
pub struct Sockets {
    /// Connections the serve loop accepted, not yet taken by the driver,
    /// and whether the last accept failed.
    accepted: VecDeque<TcpStream>,
    accept_failed: bool,
    /// The driver holds the listener open.
    listening: bool,
    streams: Vec<(u32, TcpStream)>,
    next_stream: u32,
    mdns: Option<UdpSocket>,
    /// The address the A record gives.
    ip: Ipv4Addr,
    /// The NTP exchange's socket, on an ephemeral loopback-reachable port.
    ntp: Option<std::net::UdpSocket>,
    /// Every client's window is shut: a send takes nothing.
    sends_blocked: bool,
    /// The radio is on, as the loop last asked.
    radio: bool,
}

impl Sockets {
    /// The sockets of one boot; with `mdns`, the device API is advertised
    /// on it as the given address.
    pub fn new(mdns: Option<(UdpSocket, Ipv4Addr)>) -> Self {
        let (mdns, ip) = match mdns {
            Some((socket, ip)) => (Some(socket), ip),
            None => (None, Ipv4Addr::LOCALHOST),
        };
        Self {
            accepted: VecDeque::new(),
            accept_failed: false,
            listening: false,
            streams: Vec::new(),
            next_stream: 0,
            mdns,
            ip,
            ntp: None,
            sends_blocked: false,
            radio: true,
        }
    }

    /// Every client's window shut (`true`), so no send takes a byte, or open.
    pub fn block_sends(&mut self, blocked: bool) {
        self.sends_blocked = blocked;
    }

    /// The radio on or off. Off drops every connection and the NTP socket,
    /// and takes no new one until it is on again.
    pub fn radio(&mut self, on: bool) {
        if !on && self.radio {
            self.streams.clear();
            self.accepted.clear();
            self.ntp = None;
        }
        self.radio = on;
    }

    /// The device's address while the radio is on.
    pub fn ipv4(&self) -> Option<u32> {
        self.radio.then(|| u32::from(self.ip))
    }

    /// Whether the serve loop should take connections: the driver holds
    /// the listener open, and the radio is on.
    pub fn listening(&self) -> bool {
        self.listening && self.radio
    }

    /// A connection the serve loop accepted, for the driver's next accept.
    pub fn accepted(&mut self, stream: TcpStream) {
        self.accepted.push_back(stream);
    }

    /// The serve loop's accept failed: the driver's next accept reports it
    /// and pauses the listener.
    pub fn accept_failed(&mut self) {
        self.accept_failed = true;
    }

    /// The driver opens the listener the serve loop bound before the first
    /// boot.
    pub fn listen(&mut self) -> Socket {
        self.listening = true;
        Socket::Listener
    }

    pub fn accept(&mut self) -> Accepted<Socket> {
        if !self.radio {
            self.accepted.clear();
            return Accepted::None;
        }
        if core::mem::take(&mut self.accept_failed) {
            return Accepted::Error;
        }
        let Some(stream) = self.accepted.pop_front() else {
            return Accepted::None;
        };
        let id = self.next_stream;
        self.next_stream = self.next_stream.wrapping_add(1);
        self.streams.push((id, stream));
        Accepted::Socket(Socket::Stream(id))
    }

    pub fn receive(&mut self, socket: Socket, into: &mut [u8]) -> Received {
        let Socket::Stream(id) = socket else {
            return Received::Closed;
        };
        let Some(stream) = self.stream(id) else {
            return Received::Closed;
        };
        match stream.try_read(into) {
            Ok(0) => Received::Closed,
            Ok(n) => Received::Data(n),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Received::Empty,
            Err(_) => Received::Closed,
        }
    }

    pub fn send(&mut self, socket: Socket, bytes: &[u8]) -> Option<usize> {
        let Socket::Stream(id) = socket else {
            return None;
        };
        if self.sends_blocked {
            return Some(0);
        }
        match self.stream(id)?.try_write(bytes) {
            Ok(n) => Some(n),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Some(0),
            Err(_) => None,
        }
    }

    pub fn close(&mut self, socket: Socket) {
        match socket {
            Socket::Listener => self.listening = false,
            Socket::Mdns => {}
            Socket::Ntp => self.ntp = None,
            Socket::Stream(id) => self.streams.retain(|(stream, _)| *stream != id),
        }
    }

    /// A failed send is only a lost answer: the asker asks again.
    pub fn mdns_send(&mut self, bytes: &[u8], ip: u32, port: u16) -> bool {
        if let Some(mdns) = self.mdns.as_ref().filter(|_| self.radio) {
            let to = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port));
            let _ = mdns.try_send_to(bytes, to);
        }
        true
    }

    pub fn mdns_receive(&mut self, into: &mut [u8]) -> Datagram {
        let Some(mdns) = self.mdns.as_ref().filter(|_| self.radio) else {
            return Datagram::None;
        };
        match mdns.try_recv_from(into) {
            Ok((len, SocketAddr::V4(from))) => Datagram::Got {
                len,
                ip: u32::from(*from.ip()),
                port: from.port(),
            },
            Ok(_) | Err(_) => Datagram::None,
        }
    }

    pub fn ntp_open(&mut self) -> Option<Socket> {
        if !self.radio {
            return None;
        }
        let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.set_nonblocking(true).ok()?;
        self.ntp = Some(socket);
        Some(Socket::Ntp)
    }

    /// A failed send is only a lost request: the exchange asks again.
    pub fn ntp_send(&mut self, bytes: &[u8], ip: u32, port: u16) -> bool {
        let Some(ntp) = self.ntp.as_ref().filter(|_| self.radio) else {
            return false;
        };
        let to = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        ntp.send_to(bytes, to).is_ok()
    }

    pub fn ntp_receive(&mut self, into: &mut [u8]) -> Datagram {
        let Some(ntp) = &self.ntp else {
            return Datagram::Error;
        };
        match ntp.recv_from(into) {
            Ok((len, SocketAddr::V4(from))) => Datagram::Got {
                len,
                ip: u32::from(*from.ip()),
                port: from.port(),
            },
            Ok(_) => Datagram::None,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Datagram::None,
            // A refused port (nothing listens yet) is not the socket's end.
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Datagram::None,
            Err(_) => Datagram::Error,
        }
    }

    fn stream(&mut self, id: u32) -> Option<&mut TcpStream> {
        self.streams
            .iter_mut()
            .find(|(stream, _)| *stream == id)
            .map(|(_, stream)| stream)
    }
}
