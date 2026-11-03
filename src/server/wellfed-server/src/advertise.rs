//! Announcing the server on the LAN over mDNS, so the app finds it without
//! a typed address.
//!
//! Anyone on the LAN hears the record, so it carries the HTTP port and the
//! server's version and nothing else: no household, account or feeder. The
//! app connects to the address it resolves, which the server always
//! answers to, since an IP address passes the host check. Announcing is
//! best effort: a host whose mDNS fails still serves, with a warning.

use std::net::SocketAddr;
use std::time::Duration;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Advertise;

/// The service type the app browses for.
pub(crate) const SERVICE: &str = "_wellfed-server._tcp.local.";

/// The instance and host name when the machine's own name is unusable.
const FALLBACK: &str = "wellfed-server";

/// How long the goodbye and the daemon's stop may take at shutdown.
const WITHDRAW: Duration = Duration::from_secs(2);

/// Announce the server listening at `listening` until `cancel`, then
/// withdraw the record, so the app drops it at once rather than at its
/// TTL. `None` when nothing is announced: `advertise` is off, the server
/// listens on this computer only, or mDNS failed.
pub fn spawn(
    advertise: Advertise,
    listening: SocketAddr,
    cancel: CancellationToken,
) -> Option<JoinHandle<()>> {
    if advertise == Advertise::Off {
        return None;
    }
    let record = match record(listening, &host_label(&gethostname::gethostname())) {
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

/// The record announcing a server listening at `listening` on the host
/// `label`; `None` for a loopback address, which nothing else could reach.
/// A wildcard address announces the host's addresses of its family as
/// they come and go; any other, that address alone.
fn record(listening: SocketAddr, label: &str) -> Result<Option<ServiceInfo>, mdns_sd::Error> {
    let ip = listening.ip();
    if ip.is_loopback() {
        return Ok(None);
    }

    let host = format!("{label}.local.");
    let properties = [("version", env!("CARGO_PKG_VERSION"))];
    let port = listening.port();
    let (mut record, interfaces) = if !ip.is_unspecified() {
        let record = ServiceInfo::new(SERVICE, label, &host, ip, port, &properties[..])?;
        (record, IfKind::Addr(ip))
    } else {
        let record = ServiceInfo::new(SERVICE, label, &host, (), port, &properties[..])?;
        // `[::]` takes IPv4 too on a dual-stack host; `0.0.0.0` only IPv4.
        let family = if ip.is_ipv4() {
            IfKind::IPv4
        } else {
            IfKind::All
        };
        (record.enable_addr_auto(), family)
    };
    record.set_interfaces(vec![interfaces]);
    Ok(Some(record))
}

/// The machine's name as one DNS label: its first label, lowercased, with
/// anything but letters, digits and inner hyphens dropped, at most 63
/// bytes; [`FALLBACK`] when nothing is left.
fn host_label(name: &std::ffi::OsStr) -> String {
    let name = name.to_string_lossy();
    let first = name.split('.').next().unwrap_or_default();
    let kept: String = first
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(|c| c.to_ascii_lowercase())
        .take(63)
        .collect();
    let label = kept.trim_matches('-');
    if label.is_empty() {
        FALLBACK.to_owned()
    } else {
        label.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    fn at(address: &str) -> Option<ServiceInfo> {
        record(address.parse().unwrap(), "pantry").unwrap()
    }

    #[test]
    fn a_wildcard_listener_announces_the_hosts_addresses_and_the_version_only() {
        let record = at("0.0.0.0:8080").unwrap();

        assert_eq!(record.get_type(), SERVICE);
        assert_eq!(record.get_fullname(), "pantry._wellfed-server._tcp.local.");
        assert_eq!(record.get_hostname(), "pantry.local.");
        assert_eq!(record.get_port(), 8080);
        assert!(record.is_addr_auto());
        assert!(record.get_addresses().is_empty());
        assert_eq!(
            record.get_property_val_str("version"),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(record.get_properties().len(), 1, "nothing but the version");
    }

    #[test]
    fn a_listener_on_one_address_announces_that_address() {
        let record = at("192.168.1.20:9000").unwrap();

        let want = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));
        assert_eq!(record.get_addresses().iter().collect::<Vec<_>>(), [&want]);
        assert_eq!(record.get_port(), 9000);
        assert!(!record.is_addr_auto());
    }

    #[test]
    fn a_loopback_listener_is_never_announced() {
        assert!(at("127.0.0.1:8080").is_none());
        assert!(at("[::1]:8080").is_none());
    }

    #[test]
    fn off_announces_nothing() {
        let cancel = CancellationToken::new();

        let spawned = spawn(Advertise::Off, "0.0.0.0:8080".parse().unwrap(), cancel);

        assert!(spawned.is_none());
    }

    #[test]
    fn the_host_name_becomes_one_dns_label() {
        assert_eq!(host_label(OsStr::new("Kitchen-PC.lan")), "kitchen-pc");
        assert_eq!(host_label(OsStr::new("isaac's mac")), "isaacsmac");
        assert_eq!(host_label(OsStr::new("-")), FALLBACK);
        assert_eq!(host_label(OsStr::new("")), FALLBACK);
        assert_eq!(host_label(OsStr::new(&"a".repeat(80))).len(), 63);
    }
}
