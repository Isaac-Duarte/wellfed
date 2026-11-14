//! Which network a computer is on, for the catch-up's one judgement of its
//! own: a feeder that stops answering counts as unreachable only on the
//! network where it last answered. A phone says which Wi-Fi it is on; a
//! computer may be on a cable, and no system call names its network on all
//! three desktops. So a computer's network is the IPv4 network of the
//! interface its default route leaves by: what tells home from a café, with
//! the same weakness as a phone's gateway and subnet, that a lookalike
//! network passes.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// An address the default route covers, never sent to: connecting a UDP
/// socket only chooses its route and sends nothing. TEST-NET-1, which no
/// network routes on its own.
const ANYWHERE: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);

/// This computer's network, as `address/prefix`, or `None` with no default
/// route or no IPv4 address on its interface: then it is away from every
/// feeder's home.
pub(crate) fn default_route_network() -> Option<String> {
    let local = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            socket.connect(ANYWHERE)?;
            socket.local_addr()
        })
        .ok()?;
    let SocketAddr::V4(local) = local else {
        return None;
    };
    let interfaces = if_addrs::get_if_addrs().ok()?;
    let addresses: Vec<(Ipv4Addr, u8)> = interfaces
        .iter()
        .filter_map(|interface| match &interface.addr {
            if_addrs::IfAddr::V4(v4) => Some((v4.ip, v4.prefixlen)),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    network_of(*local.ip(), &addresses)
}

/// The network of the interface holding `local`, among `addresses` (each
/// an interface's address and prefix length).
fn network_of(local: Ipv4Addr, addresses: &[(Ipv4Addr, u8)]) -> Option<String> {
    if local.is_unspecified() || local.is_loopback() {
        return None;
    }
    let &(_, prefix) = addresses.iter().find(|(ip, _)| *ip == local)?;
    let mask = u32::MAX
        .checked_shl(32 - u32::from(prefix.min(32)))
        .unwrap_or(0);
    let network = Ipv4Addr::from(u32::from(local) & mask);
    Some(format!("{network}/{prefix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 23);

    #[test]
    fn the_network_is_the_interface_holding_the_routes_address() {
        let addresses = [
            (Ipv4Addr::LOCALHOST, 8),
            (Ipv4Addr::new(172, 17, 0, 1), 16),
            (HOME, 24),
        ];

        assert_eq!(
            network_of(HOME, &addresses).as_deref(),
            Some("192.168.1.0/24")
        );
        assert_eq!(
            network_of(
                Ipv4Addr::new(10, 1, 2, 3),
                &[(Ipv4Addr::new(10, 1, 2, 3), 8)]
            )
            .as_deref(),
            Some("10.0.0.0/8")
        );
    }

    #[test]
    fn another_address_on_the_same_network_is_the_same_network() {
        let other = Ipv4Addr::new(192, 168, 1, 77);

        assert_eq!(
            network_of(HOME, &[(HOME, 24)]),
            network_of(other, &[(other, 24)])
        );
    }

    #[test]
    fn no_route_or_no_interface_is_no_network() {
        assert_eq!(
            network_of(Ipv4Addr::UNSPECIFIED, &[(Ipv4Addr::UNSPECIFIED, 0)]),
            None
        );
        assert_eq!(
            network_of(Ipv4Addr::LOCALHOST, &[(Ipv4Addr::LOCALHOST, 8)]),
            None
        );
        assert_eq!(
            network_of(HOME, &[(Ipv4Addr::new(192, 168, 1, 24), 24)]),
            None
        );
    }

    #[test]
    fn edge_prefixes_hold() {
        assert_eq!(
            network_of(HOME, &[(HOME, 32)]).as_deref(),
            Some("192.168.1.23/32")
        );
        assert_eq!(network_of(HOME, &[(HOME, 0)]).as_deref(), Some("0.0.0.0/0"));
    }
}
