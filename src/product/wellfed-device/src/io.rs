//! What a device's driver reports from its sockets and its setup link,
//! whatever the device: every kind's loop takes the same answers from the
//! firmware's lwIP and BLE drivers and from a simulator's host sockets.

/// What a non-blocking accept found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted<S> {
    Socket(S),
    /// No connection waits.
    None,
    /// The listener failed; open a new one.
    Error,
}

/// What a non-blocking receive found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Received {
    Data(usize),
    /// Nothing waiting now.
    Empty,
    /// The peer closed the connection, or the socket failed.
    Closed,
}

/// One mDNS datagram, if one waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Datagram {
    Got { len: usize, ip: u32, port: u16 },
    None,
    Error,
}

/// What setup mode's BLE link asks the loop to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BleLink {
    /// A host connected and enabled notifications: a new session.
    Opened,
    /// A whole message waits for the loop to take.
    Message,
    /// The host left or was dropped: forget its session.
    Closed,
}

/// Why setup mode drops its BLE host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BleDrop {
    /// The session ended; its last reply has had time to go out.
    Done,
    /// The host broke the protocol.
    Protocol,
}
