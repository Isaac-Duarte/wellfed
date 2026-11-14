//! Which network a computer is on, for the catch-up's one judgement of its
//! own: a feeder that stops answering counts as unreachable only on the
//! network where it last answered. A phone says which Wi-Fi it is on; a
//! computer may be on a cable, and no system call names its network on all
//! three desktops. So a computer's network is the IPv4 network of the
//! interface its default route leaves by: what tells home from a café, with
//! the same weakness as a phone's gateway and subnet, that a lookalike
//! network passes.
//!
//! A full-tunnel VPN takes the default route, and its address is the same
//! wherever the computer is, so it would pass for home anywhere and count
//! failures abroad. Past a tunnel the network is the one LAN interface the
//! computer has; with none or several, it can't be named, and the catch-up
//! still dials but judges nothing.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// An address the default route covers, never sent to: connecting a UDP
/// socket only chooses its route and sends nothing. TEST-NET-1, which no
/// network routes on its own.
const ANYWHERE: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);

/// Interface names that are tunnels whatever their addresses say: WireGuard,
/// TUN devices, macOS's utun, PPP, IPsec, and the VPN clients that name
/// their own.
const TUNNELS: &[&str] = &[
    "wg",
    "tun",
    "utun",
    "ppp",
    "ipsec",
    "tailscale",
    "zt",
    "nordlynx",
    "proton",
    "mullvad",
];

/// Interface names of bridges a computer makes for its own guests, which sit
/// on private networks without being any network the computer is on.
const GUEST_BRIDGES: &[&str] = &[
    "docker",
    "br-",
    "veth",
    "virbr",
    "vmnet",
    "vboxnet",
    "lxc",
    "lxdbr",
    "podman",
    "cni",
    "vEthernet",
];

/// Where the computer is, as its interfaces say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// On the IPv4 network `address/prefix`.
    Network(String),
    /// Online, but on no network that can be named: a tunnel with no single
    /// LAN beside it.
    Unnamed,
    /// No default IPv4 route, or no address on its interface.
    None,
}

/// One IPv4 address of an interface, as the system lists it.
#[derive(Clone, Debug)]
struct Address {
    name: String,
    ip: Ipv4Addr,
    prefix: u8,
    point_to_point: bool,
}

/// This computer's network.
pub(crate) fn default_route() -> Route {
    let Ok(local) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|socket| {
        socket.connect(ANYWHERE)?;
        socket.local_addr()
    }) else {
        return Route::None;
    };
    let SocketAddr::V4(local) = local else {
        return Route::None;
    };
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Route::None;
    };
    let addresses: Vec<Address> = interfaces
        .iter()
        .filter_map(|interface| match &interface.addr {
            if_addrs::IfAddr::V4(v4) => Some(Address {
                name: interface.name.clone(),
                ip: v4.ip,
                prefix: v4.prefixlen,
                point_to_point: interface.is_p2p(),
            }),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    route_of(*local.ip(), &addresses)
}

/// The network of the interface holding `local`, among `addresses`, or past
/// a tunnel the one LAN beside it.
fn route_of(local: Ipv4Addr, addresses: &[Address]) -> Route {
    if local.is_unspecified() || local.is_loopback() {
        return Route::None;
    }
    let Some(routed) = addresses.iter().find(|address| address.ip == local) else {
        return Route::None;
    };
    if !tunnel(routed) {
        return Route::Network(network(routed));
    }

    let mut lans = addresses.iter().filter(|address| {
        address.ip.is_private() && !tunnel(address) && !named(&address.name, GUEST_BRIDGES)
    });
    match (lans.next(), lans.next()) {
        (Some(lan), None) => Route::Network(network(lan)),
        _ => Route::Unnamed,
    }
}

fn tunnel(address: &Address) -> bool {
    address.point_to_point || address.prefix >= 32 || named(&address.name, TUNNELS)
}

fn named(name: &str, prefixes: &[&str]) -> bool {
    let name = name.to_ascii_lowercase();
    prefixes
        .iter()
        .any(|prefix| name.starts_with(&prefix.to_ascii_lowercase()))
}

fn network(address: &Address) -> String {
    let prefix = address.prefix.min(32);
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
    let network = Ipv4Addr::from(u32::from(address.ip) & mask);
    format!("{network}/{prefix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 23);

    fn at(name: &str, ip: Ipv4Addr, prefix: u8) -> Address {
        Address {
            name: name.into(),
            ip,
            prefix,
            point_to_point: false,
        }
    }

    fn lan(network: &str) -> Route {
        Route::Network(network.into())
    }

    #[test]
    fn the_network_is_the_interface_holding_the_routes_address() {
        let addresses = [
            at("lo", Ipv4Addr::LOCALHOST, 8),
            at("docker0", Ipv4Addr::new(172, 17, 0, 1), 16),
            at("wlan0", HOME, 24),
        ];

        assert_eq!(route_of(HOME, &addresses), lan("192.168.1.0/24"));
        let ten = Ipv4Addr::new(10, 1, 2, 3);
        assert_eq!(route_of(ten, &[at("eth0", ten, 8)]), lan("10.0.0.0/8"));
    }

    #[test]
    fn another_address_on_the_same_network_is_the_same_network() {
        let other = Ipv4Addr::new(192, 168, 1, 77);

        assert_eq!(
            route_of(HOME, &[at("eth0", HOME, 24)]),
            route_of(other, &[at("en0", other, 24)])
        );
    }

    #[test]
    fn no_route_or_no_interface_is_no_network() {
        let unspecified = Ipv4Addr::UNSPECIFIED;
        assert_eq!(
            route_of(unspecified, &[at("eth0", unspecified, 0)]),
            Route::None
        );
        assert_eq!(
            route_of(Ipv4Addr::LOCALHOST, &[at("lo", Ipv4Addr::LOCALHOST, 8)]),
            Route::None
        );
        assert_eq!(
            route_of(HOME, &[at("eth0", Ipv4Addr::new(192, 168, 1, 24), 24)]),
            Route::None
        );
    }

    #[test]
    fn a_full_tunnel_vpn_names_the_lan_beside_it_not_itself() {
        let tunnel = Ipv4Addr::new(10, 64, 0, 2);
        let addresses = [
            at("wg0", tunnel, 24),
            at("docker0", Ipv4Addr::new(172, 17, 0, 1), 16),
            at("wlan0", HOME, 24),
        ];

        assert_eq!(route_of(tunnel, &addresses), lan("192.168.1.0/24"));
    }

    #[test]
    fn a_tunnel_is_known_by_its_name_its_prefix_or_its_point_to_point_link() {
        let tunnel = Ipv4Addr::new(10, 64, 0, 2);
        let wifi = at("Wi-Fi", HOME, 24);
        let host_route = [at("VPN", tunnel, 32), wifi.clone()];
        let point_to_point = [
            Address {
                point_to_point: true,
                ..at("VPN", tunnel, 24)
            },
            wifi.clone(),
        ];
        let utun = [at("utun3", tunnel, 24), wifi];

        for addresses in [&host_route[..], &point_to_point[..], &utun[..]] {
            assert_eq!(
                route_of(tunnel, addresses),
                lan("192.168.1.0/24"),
                "{addresses:?}"
            );
        }
    }

    #[test]
    fn past_a_tunnel_no_lan_or_two_is_a_network_with_no_name() {
        let tunnel = Ipv4Addr::new(10, 64, 0, 2);

        assert_eq!(route_of(tunnel, &[at("wg0", tunnel, 24)]), Route::Unnamed);
        let two = [
            at("wg0", tunnel, 24),
            at("eth0", Ipv4Addr::new(192, 168, 1, 5), 24),
            at("wlan0", Ipv4Addr::new(10, 0, 0, 5), 24),
        ];
        assert_eq!(route_of(tunnel, &two), Route::Unnamed);
        let public = [
            at("wg0", tunnel, 24),
            at("eth0", Ipv4Addr::new(203, 0, 113, 5), 24),
        ];
        assert_eq!(
            route_of(tunnel, &public),
            Route::Unnamed,
            "a public address is no home LAN"
        );
    }

    #[test]
    fn edge_prefixes_hold() {
        assert_eq!(
            route_of(HOME, &[at("eth0", HOME, 31)]),
            lan("192.168.1.22/31")
        );
        assert_eq!(route_of(HOME, &[at("eth0", HOME, 0)]), lan("0.0.0.0/0"));
    }
}
