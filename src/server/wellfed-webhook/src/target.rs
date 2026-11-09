//! Where deliveries go: an `https://` URL, or plain `http://` only to a
//! receiver on this computer or the home network, unless the owner opts in.
//! A webhook URL is often a secret of its own (Home Assistant's is), and
//! plain HTTP across the internet would show it and every alert to anyone
//! on the way, so that takes the owner's say-so, as Home Assistant's own
//! plain-`http://` server address does.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::Url;

/// Whether a plain `http://` URL to a host off the home network is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlainHttp {
    #[default]
    Refused,
    /// The owner said so (`webhook.allow_plain_http`).
    Allowed,
}

/// A checked webhook URL. It never prints whole, since its path may be a
/// secret; only its host does.
#[derive(Clone, PartialEq, Eq)]
pub struct Target {
    url: Url,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    #[error("a webhook URL is https:// or http://, not {scheme}://")]
    Scheme { scheme: String },
    #[error("a webhook URL names a host")]
    NoHost,
    #[error(
        "plain http:// to {host}, off this computer and the home network, would show every \
         alert on the way: use https://, or set `webhook.allow_plain_http`"
    )]
    PlainHttp { host: String },
}

impl Target {
    pub fn new(url: Url, plain: PlainHttp) -> Result<Self, TargetError> {
        let host = url.host_str().ok_or(TargetError::NoHost)?.to_owned();
        match url.scheme() {
            "https" => {}
            "http" if plain == PlainHttp::Allowed || at_home(&host) => {}
            "http" => return Err(TargetError::PlainHttp { host }),
            scheme => {
                return Err(TargetError::Scheme {
                    scheme: scheme.to_owned(),
                });
            }
        }
        Ok(Self { url })
    }

    pub(crate) const fn url(&self) -> &Url {
        &self.url
    }

    /// The host deliveries go to, for logs.
    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or_default()
    }
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("host", &self.host())
            .finish_non_exhaustive()
    }
}

/// Whether `host` is this computer or on the home network: a loopback,
/// private or link-local address, `localhost`, or an mDNS `.local` name,
/// which resolves only on the local link.
fn at_home(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".local") {
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

    fn target(url: &str, plain: PlainHttp) -> Result<Target, TargetError> {
        Target::new(Url::parse(url).expect("a URL"), plain)
    }

    #[test]
    fn https_goes_anywhere() {
        assert!(target("https://example.com/hook", PlainHttp::Refused).is_ok());
    }

    #[test]
    fn plain_http_stays_home_unless_allowed() {
        for home in [
            "http://127.0.0.1:8123/api/webhook/x",
            "http://192.168.1.20:8123/api/webhook/x",
            "http://10.0.0.2/x",
            "http://172.16.4.4/x",
            "http://[::1]/x",
            "http://[fd00::4]/x",
            "http://homeassistant.local:8123/api/webhook/x",
            "http://localhost/x",
        ] {
            assert!(target(home, PlainHttp::Refused).is_ok(), "{home}");
        }
        for away in [
            "http://example.com/x",
            "http://8.8.8.8/x",
            "http://[2001:db8::1]/x",
        ] {
            assert!(
                matches!(
                    target(away, PlainHttp::Refused),
                    Err(TargetError::PlainHttp { .. })
                ),
                "{away}"
            );
            assert!(target(away, PlainHttp::Allowed).is_ok(), "{away}");
        }
    }

    #[test]
    fn other_schemes_are_refused() {
        assert_eq!(
            target("ftp://192.168.1.2/x", PlainHttp::Allowed),
            Err(TargetError::Scheme {
                scheme: "ftp".to_owned()
            })
        );
    }

    #[test]
    fn the_url_never_prints() {
        let hook =
            target("https://example.com/api/webhook/secret", PlainHttp::Refused).expect("taken");
        assert!(!format!("{hook:?}").contains("secret"));
    }
}
