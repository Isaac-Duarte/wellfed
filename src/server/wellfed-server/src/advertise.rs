//! Announcing the server on the LAN over mDNS, so the app finds it without
//! a typed address.
//!
//! Anyone on the LAN hears the record, so it carries the port of the
//! server's own HTTPS (`tls_listen`), the server's version, its
//! [`ServerId`] and its identity key, and nothing else: no household,
//! account, feeder or machine name. The plain HTTP port is never announced:
//! the app reaches the server on the home network only over HTTPS, pinning
//! the key the record names, so a server without `tls_listen` announces
//! nothing. The instance and the host are the server's `label` and the
//! id's first digits, so two servers on the default label never announce
//! one name, and the host never collides with the machine's own mDNS name.
//! The app connects to the address it resolves, which the server always
//! answers to, since an IP address passes the host check, and recognises
//! the server by `id` when that address changes. A record anyone can forge
//! is safe to follow because of the key: a host that announces another
//! server's key cannot finish its handshake. Announcing is best effort: a
//! host whose mDNS fails still serves, with a warning.

use std::net::IpAddr;
use std::time::Duration;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Advertise;
use crate::server_id::ServerId;

/// The service type the app browses for.
pub(crate) const SERVICE: &str = "_wellfed-server._tcp.local.";

/// The host name's stem when the label leaves no DNS label.
const FALLBACK: &str = "wellfed-server";

/// How long the goodbye and the daemon's stop may take at shutdown.
const WITHDRAW: Duration = Duration::from_secs(2);

/// Announce the server's HTTPS on `listener` as `label`, with its identity
/// `key`, until `cancel`, then withdraw the record, so the app drops it at
/// once rather than at its TTL. `None` when nothing is announced:
/// `advertise` is off, there is no HTTPS listener, the server listens on
/// this computer only, or mDNS failed.
pub fn spawn(
    advertise: Advertise,
    listener: Option<&TcpListener>,
    label: &str,
    id: ServerId,
    key: [u8; 32],
    cancel: CancellationToken,
) -> Option<JoinHandle<()>> {
    if advertise == Advertise::Off {
        return None;
    }
    let Some(listener) = listener else {
        info!("not advertised over mDNS: the app finds the server only over HTTPS (`tls_listen`)");
        return None;
    };
    let listening = match listener.local_addr() {
        Ok(listening) => listening,
        Err(error) => {
            warn!("not advertised over mDNS: {error}");
            return None;
        }
    };
    // A socket on `[::]` takes IPv4 too unless it is IPv6-only; an unknown
    // answer announces IPv6 alone rather than an address family it may not
    // serve.
    let v6_only = listening.is_ipv6() && socket2::SockRef::from(listener).only_v6().unwrap_or(true);
    let serves = serves(listening.ip(), v6_only);
    let record = match record(serves, listening.port(), label, id, &key) {
        Ok(Some(record)) => record,
        Ok(None) => {
            info!("not advertised over mDNS: {listening} is this computer only");
            return None;
        }
        Err(error) => {
            warn!("not advertised over mDNS: {error}");
            return None;
        }
    };

    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            warn!("not advertised over mDNS: {error}");
            return None;
        }
    };
    let fullname = record.get_fullname().to_owned();
    if let Err(error) = daemon.register(record) {
        warn!("not advertised over mDNS: {error}");
        // Stopping only ends the daemon's thread: nothing was announced.
        let _ = daemon.shutdown();
        return None;
    }
    info!("advertising {fullname} over mDNS");

    Some(tokio::spawn(async move {
        cancel.cancelled().await;
        withdraw(&daemon, &fullname).await;
    }))
}

/// Send the record's goodbye, then stop the daemon, each for at most
/// [`WITHDRAW`]: a stuck daemon never holds up the shutdown.
async fn withdraw(daemon: &ServiceDaemon, fullname: &str) {
    match daemon.unregister(fullname) {
        Ok(status) => {
            if tokio::time::timeout(WITHDRAW, status.recv_async())
                .await
                .is_err()
            {
                warn!("the mDNS goodbye for {fullname} was not confirmed");
            }
        }
        Err(error) => warn!("withdraw {fullname} from mDNS: {error}"),
    }
    match daemon.shutdown() {
        Ok(status) => {
            let _ = tokio::time::timeout(WITHDRAW, status.recv_async()).await;
        }
        Err(error) => warn!("stop the mDNS daemon: {error}"),
    }
}

/// The addresses a listening socket serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Serves {
    /// This computer only.
    Loopback,
    One(IpAddr),
    /// Every address of the host in a family, as they come and go.
    AllV4,
    AllV6,
    AllBoth,
}

/// What a socket bound to `ip` serves; `v6_only` says whether one on `[::]`
/// refuses IPv4.
fn serves(ip: IpAddr, v6_only: bool) -> Serves {
    match ip {
        _ if ip.is_loopback() => Serves::Loopback,
        IpAddr::V4(_) if ip.is_unspecified() => Serves::AllV4,
        IpAddr::V6(_) if ip.is_unspecified() && v6_only => Serves::AllV6,
        IpAddr::V6(_) if ip.is_unspecified() => Serves::AllBoth,
        _ => Serves::One(ip),
    }
}

/// The record announcing the server as `label`, its HTTPS on `port`;
/// `None` on loopback, which nothing else could reach.
fn record(
    serves: Serves,
    port: u16,
    label: &str,
    id: ServerId,
    key: &[u8; 32],
) -> Result<Option<ServiceInfo>, mdns_sd::Error> {
    let instance = format!("{label}-{}", id.short());
    let host = format!("{}-{}.local.", host_stem(label), id.short());
    let id = id.to_string();
    let key = wellfed_api::Hex(*key).to_string();
    let properties = [
        ("version", env!("CARGO_PKG_VERSION")),
        ("id", id.as_str()),
        (wellfed_api::TXT_KEY, key.as_str()),
    ];
    let auto = |family| -> Result<_, mdns_sd::Error> {
        let record = ServiceInfo::new(SERVICE, &instance, &host, (), port, &properties[..])?;
        Ok((record.enable_addr_auto(), family))
    };

    let (mut record, interfaces) = match serves {
        Serves::Loopback => return Ok(None),
        Serves::One(ip) => {
            let record = ServiceInfo::new(SERVICE, &instance, &host, ip, port, &properties[..])?;
            (record, IfKind::Addr(ip))
        }
        Serves::AllV4 => auto(IfKind::IPv4)?,
        Serves::AllV6 => auto(IfKind::IPv6)?,
        Serves::AllBoth => auto(IfKind::All)?,
    };
    record.set_interfaces(vec![interfaces]);
    Ok(Some(record))
}

/// The label as one DNS label: lowercased, with anything but letters,
/// digits and inner hyphens dropped; [`FALLBACK`] when nothing is left. A
/// label is at most 16 bytes, so the host name stays short.
fn host_stem(label: &str) -> String {
    let kept: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let stem = kept.trim_matches('-');
    if stem.is_empty() {
        FALLBACK.to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn id() -> ServerId {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join(crate::server_id::ID_FILE),
            "0123456789abcdef",
        )
        .unwrap();
        crate::server_id::server_id(data.path(), &wellfed_service::SystemRandom).unwrap()
    }

    fn at(serves: Serves) -> Option<ServiceInfo> {
        record(serves, 8443, "wellfed-server", id(), &[0xab; 32]).unwrap()
    }

    #[test]
    fn the_record_names_the_label_a_host_of_its_own_the_version_the_id_and_the_key() {
        let record = at(Serves::AllV4).unwrap();

        assert_eq!(record.get_type(), SERVICE);
        assert_eq!(
            record.get_fullname(),
            "wellfed-server-01234567._wellfed-server._tcp.local."
        );
        assert_eq!(record.get_hostname(), "wellfed-server-01234567.local.");
        assert_eq!(record.get_port(), 8443);
        assert_eq!(
            record.get_property_val_str("version"),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(record.get_property_val_str("id"), Some("0123456789abcdef"));
        assert_eq!(
            record.get_property_val_str("key"),
            Some("ab".repeat(32).as_str())
        );
        assert_eq!(
            record.get_properties().len(),
            3,
            "nothing but the version, the id and the key"
        );
    }

    #[test]
    fn a_wildcard_listener_announces_the_hosts_addresses() {
        for serves in [Serves::AllV4, Serves::AllV6, Serves::AllBoth] {
            let record = at(serves).unwrap();

            assert!(record.is_addr_auto(), "{serves:?}");
            assert!(record.get_addresses().is_empty(), "{serves:?}");
        }
    }

    #[test]
    fn a_listener_on_one_address_announces_that_address() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

        let record = at(Serves::One(ip)).unwrap();

        assert_eq!(record.get_addresses().iter().collect::<Vec<_>>(), [&ip]);
        assert!(!record.is_addr_auto());
    }

    #[test]
    fn a_loopback_listener_is_never_announced() {
        assert!(at(Serves::Loopback).is_none());
    }

    #[test]
    fn what_a_socket_serves_follows_its_address_and_family() {
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();

        assert_eq!(serves(ip("127.0.0.1"), false), Serves::Loopback);
        assert_eq!(serves(ip("::1"), true), Serves::Loopback);
        assert_eq!(serves(ip("0.0.0.0"), false), Serves::AllV4);
        assert_eq!(serves(ip("::"), true), Serves::AllV6);
        assert_eq!(serves(ip("::"), false), Serves::AllBoth);
        let lan = ip("192.168.1.20");
        assert_eq!(serves(lan, false), Serves::One(lan));
    }

    #[test]
    fn off_announces_nothing() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();

            let spawned = spawn(
                Advertise::Off,
                Some(&listener),
                "wellfed-server",
                id(),
                [0; 32],
                CancellationToken::new(),
            );

            assert!(spawned.is_none());
        });
    }

    #[test]
    fn no_https_announces_nothing() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let spawned = spawn(
                Advertise::Mdns,
                None,
                "wellfed-server",
                id(),
                [0; 32],
                CancellationToken::new(),
            );

            assert!(spawned.is_none());
        });
    }

    #[test]
    fn the_label_becomes_one_dns_label() {
        assert_eq!(host_stem("Kitchen Server"), "kitchenserver");
        assert_eq!(host_stem("-pantry-"), "pantry");
        assert_eq!(host_stem("über"), "ber");
        assert_eq!(host_stem("—"), FALLBACK);
    }
}
