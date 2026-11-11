//! Where plain `http://` may go: to this computer or the home network, and
//! anywhere else only with the owner's say-so. The rule is the server's for
//! its webhook and every client's for the server's own address, since both
//! carry a secret (a webhook path, a password, a token that can feed) that
//! plain HTTP across the internet shows to anyone on the way.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether a plain `http://` URL to a host off the home network is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlainHttp {
    #[default]
    Refused,
    /// The owner said so, knowing what crosses the network in the clear.
    Allowed,
}

/// Whether `host` (a URL's host, IPv6 in brackets or not) is this computer
/// or on the home network: a loopback, private or link-local address,
/// `localhost`, or an mDNS `.local` name, which resolves only on the local
/// link. Any other name is not, even one a home router resolves, since
/// nothing here sees where it resolves to.
pub fn at_home(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let lower = host.to_ascii_lowercase();
    let name = lower.trim_end_matches('.');
    if name == "localhost" || name.ends_with(".local") {
        return true;
    }

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => v4_at_home(address),
        Ok(IpAddr::V6(address)) => match address.to_ipv4_mapped() {
            Some(mapped) => v4_at_home(mapped),
            None => v6_at_home(address),
        },
        Err(_) => false,
    }
}

const fn v4_at_home(address: Ipv4Addr) -> bool {
    address.is_loopback() || address.is_private() || address.is_link_local()
}

const fn v6_at_home(address: Ipv6Addr) -> bool {
    address.is_loopback() || address.is_unique_local() || address.is_unicast_link_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_loopback_private_link_local_and_mdns() {
        for home in [
            "127.0.0.1",
            "192.168.1.20",
            "10.0.0.2",
            "172.16.4.4",
            "169.254.3.3",
            "[::1]",
            "::1",
            "[fd00::4]",
            "[fe80::1]",
            "[::ffff:192.168.1.2]",
            "homeassistant.local",
            "Server.LOCAL.",
            "localhost",
        ] {
            assert!(at_home(home), "{home}");
        }
        for away in [
            "example.com",
            "feeder.example",
            "nas",
            "8.8.8.8",
            "192.0.2.7",
            "100.64.0.1",
            "[2001:db8::1]",
            "[::ffff:8.8.8.8]",
            "local",
            "evil.local.example.com",
        ] {
            assert!(!at_home(away), "{away}");
        }
    }
}
