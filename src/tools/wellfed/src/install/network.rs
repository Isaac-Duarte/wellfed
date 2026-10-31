//! Whether this computer is on the feeder's setup network, and getting it there.
//!
//! The setup frame carries the Wi-Fi password in the clear to 192.168.1.1, and
//! many home routers are 192.168.1.1 too. So a frame goes only after [`look`]
//! has found the route to that address on a network named like the feeder's,
//! or the person has named such a network where this computer cannot tell. The
//! decisions here are pure; what they read and change on this computer sits
//! behind [`Host`], so a test fakes it and no test touches a real network.
//!
//! [`Nmcli`] is that seam on Linux with NetworkManager. A profile it joins with
//! carries two settings the feeder's access point needs: NetworkManager's
//! address-conflict detection reports `acd conflict` on the offered
//! 192.168.1.100, so a plain join times out (physical, October 31, 2026), and
//! `ipv4.dad-timeout 0` turns that off. `connection.autoconnect no` keeps the
//! profile from ever coming up by itself. Route settings keep the feeder's
//! address on this link when another interface holds the same subnet, and the
//! default route where it was.

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;

use wellfed_client::install::{SETUP_ADDRESS, SETUP_SSID_PREFIX};

/// The profile a hop creates, removed when it leaves.
pub(crate) const PROFILE: &str = "wellfed-feeder-setup";
/// How long NetworkManager gets to bring a connection up.
const UP_WAIT_SECONDS: &str = "30";

/// Why no frame goes: what the check found, and that nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FrameRefused {
    #[error(
        "this computer reaches {SETUP_ADDRESS} through {interface}, on the Wi-Fi network {ssid:?}, not the feeder's network ({SETUP_SSID_PREFIX}…); nothing was sent"
    )]
    OtherNetwork { interface: String, ssid: String },
    #[error(
        "this computer reaches {SETUP_ADDRESS} through {interface}, which is not Wi-Fi, so that is not the feeder's network; nothing was sent"
    )]
    NotWifi { interface: String },
    #[error(
        "this computer reaches {SETUP_ADDRESS} through the gateway {gateway} on {interface}, and the feeder's network has no gateway; nothing was sent"
    )]
    ThroughGateway { interface: String, gateway: IpAddr },
    #[error(
        "{SETUP_ADDRESS} is this computer's own address, on {interface}, not the feeder's; nothing was sent"
    )]
    ThisComputer { interface: String },
    #[error(
        "this computer has no route to {SETUP_ADDRESS}, so it is not on the feeder's network; nothing was sent"
    )]
    NoRoute,
    #[error(
        "{typed:?} is not the feeder's network, whose name starts {SETUP_SSID_PREFIX}; nothing was sent"
    )]
    NotConfirmed { typed: String },
}

/// One command this computer ran that did not do its job.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HostError {
    #[error("{tool} did not run")]
    Missing {
        tool: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("{command} failed: {said}")]
    Failed { command: String, said: String },
}

/// Where the route to an address goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Route {
    pub(crate) interface: String,
    pub(crate) gateway: Option<IpAddr>,
    /// The address is this computer's own.
    pub(crate) local: bool,
}

/// What an interface is connected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Link {
    Wifi(String),
    NotWifi,
    /// This computer cannot tell: no NetworkManager, or it does not manage
    /// this interface.
    Unknown,
}

/// A Wi-Fi interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Radio {
    pub(crate) interface: String,
    /// The connection it carries now.
    pub(crate) connection: Option<String>,
    /// That connection is an access point this computer serves.
    pub(crate) hotspot: bool,
}

/// A hop to the feeder's network: where this computer goes back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hop {
    pub(crate) interface: String,
    pub(crate) previous: Option<String>,
}

/// What reading and changing this computer's network takes.
pub(crate) trait Host {
    /// The route to `to`; `None` when there is none, an error when this
    /// computer cannot tell.
    fn route(&self, to: Ipv4Addr) -> Result<Option<Route>, HostError>;

    fn link(&self, interface: &str) -> Link;

    fn radios(&self) -> Result<Vec<Radio>, HostError>;

    /// The Wi-Fi networks `interface` hears, after a fresh scan.
    fn scan(&self, interface: &str) -> Result<Vec<String>, HostError>;

    /// Join the open network `ssid`, with a profile of its own.
    fn join(&self, hop: &Hop, ssid: &str) -> Result<(), HostError>;

    /// Remove the hop's profile, and bring back the connection it replaced.
    fn leave(&self, hop: &Hop) -> Result<(), HostError>;

    /// This computer's address on the Wi-Fi network `ssid`, as a client or as
    /// its hotspot.
    fn address_on(&self, ssid: &str) -> Option<Ipv4Addr>;
}

/// What the check before the frame found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Look {
    /// On the feeder's network: the frame may go.
    Feeder {
        ssid: String,
    },
    Refused(FrameRefused),
    /// This computer cannot name the network, so the person must.
    Ask,
}

/// Where the frame would go now. Anything but the feeder's network is a
/// refusal or a question, never an assumption.
pub(crate) fn look(host: &impl Host) -> Look {
    let route = match host.route(*SETUP_ADDRESS.ip()) {
        Ok(Some(route)) => route,
        Ok(None) => return Look::Refused(FrameRefused::NoRoute),
        Err(_) => return Look::Ask,
    };
    let interface = route.interface.clone();
    if route.local {
        return Look::Refused(FrameRefused::ThisComputer { interface });
    }
    if let Some(gateway) = route.gateway {
        return Look::Refused(FrameRefused::ThroughGateway { interface, gateway });
    }
    match host.link(&route.interface) {
        Link::Wifi(ssid) if ssid.starts_with(SETUP_SSID_PREFIX) => Look::Feeder { ssid },
        Link::Wifi(ssid) => Look::Refused(FrameRefused::OtherNetwork { interface, ssid }),
        Link::NotWifi => Look::Refused(FrameRefused::NotWifi { interface }),
        Link::Unknown => Look::Ask,
    }
}

/// The name a person gave for the network this computer is on.
pub(crate) fn confirmed(typed: &str) -> Result<String, FrameRefused> {
    let typed = typed.trim();
    if typed.starts_with(SETUP_SSID_PREFIX) {
        Ok(typed.to_string())
    } else {
        Err(FrameRefused::NotConfirmed {
            typed: typed.to_string(),
        })
    }
}

/// The interface to hop on: one that carries nothing, or else one that carries
/// something other than a hotspot, since the feeder must keep reaching that.
pub(crate) fn pick(radios: &[Radio]) -> Option<&Radio> {
    radios
        .iter()
        .find(|radio| radio.connection.is_none())
        .or_else(|| radios.iter().find(|radio| !radio.hotspot))
}

/// The feeder networks among `heard`, each once.
pub(crate) fn feeders(heard: &[String]) -> Vec<String> {
    let mut found: Vec<String> = heard
        .iter()
        .filter(|ssid| ssid.starts_with(SETUP_SSID_PREFIX))
        .cloned()
        .collect();
    found.sort();
    found.dedup();
    found
}

/// `nmcli`'s arguments for the hop's profile.
pub(crate) fn profile_args(interface: &str, ssid: &str) -> Vec<String> {
    [
        "connection",
        "add",
        "type",
        "wifi",
        "ifname",
        interface,
        "con-name",
        PROFILE,
        "ssid",
        ssid,
        "connection.autoconnect",
        "no",
        "ipv4.method",
        "auto",
        "ipv4.dad-timeout",
        "0",
        "ipv4.never-default",
        "yes",
        "ipv4.route-metric",
        "1",
        "ipv6.method",
        "disabled",
    ]
    .map(str::to_string)
    .to_vec()
}

/// The steps for a person, naming what this computer knows.
pub(crate) fn manual_steps(interface: Option<&str>, ssid: Option<&str>) -> String {
    let network = ssid.map_or_else(|| format!("{SETUP_SSID_PREFIX}…"), str::to_string);
    let mut steps = format!(
        "Join this computer to the feeder's own Wi-Fi network, {network}, on a Wi-Fi interface \
         that is not serving a hotspot."
    );
    if let (Some(interface), Some(ssid)) = (interface, ssid) {
        let add = profile_args(interface, ssid)
            .iter()
            .map(|arg| quoted(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            steps,
            " With NetworkManager:\n\n    nmcli {add}\n    nmcli connection up {PROFILE}\n\n\
             Afterwards, remove it with: nmcli connection delete {PROFILE}"
        );
    }
    steps
}

/// `arg` as a shell reads it back.
pub(crate) fn quoted(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,@+".contains(&b));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// `ip -o route get`'s one line.
fn parse_route(line: &str) -> Option<Route> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let after = |name: &str| {
        words
            .iter()
            .position(|word| *word == name)
            .and_then(|at| words.get(at + 1))
    };
    Some(Route {
        interface: (*after("dev")?).to_string(),
        gateway: after("via").and_then(|via| via.parse().ok()),
        local: words.first() == Some(&"local"),
    })
}

/// One `nmcli --terse` line's fields: `:` separates them, and a backslash
/// escapes `:` and itself inside one.
fn terse_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let (Some(next), Some(field)) = (chars.next(), fields.last_mut()) {
                    field.push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => {
                if let Some(field) = fields.last_mut() {
                    field.push(c);
                }
            }
        }
    }
    fields
}

/// A connection's name as `nmcli` prints it: nothing or `--` for none.
fn named(field: &str) -> Option<String> {
    (!field.is_empty() && field != "--").then(|| field.to_string())
}

/// The Wi-Fi interfaces in `nmcli -t -f DEVICE,TYPE,CONNECTION device`, with
/// whether each carries a hotspot asked of `mode`.
fn parse_radios(listing: &str, mode: impl Fn(&str) -> Option<String>) -> Vec<Radio> {
    listing
        .lines()
        .filter_map(|line| match terse_fields(line).as_slice() {
            [interface, kind, connection] if kind == "wifi" => {
                let connection = named(connection);
                let hotspot = connection
                    .as_deref()
                    .is_some_and(|name| mode(name).as_deref() == Some("ap"));
                Some(Radio {
                    interface: interface.clone(),
                    connection,
                    hotspot,
                })
            }
            _ => None,
        })
        .collect()
}

/// [`Host`] through `ip` and NetworkManager's `nmcli`.
pub(crate) struct Nmcli;

impl Nmcli {
    fn run(tool: &'static str, args: &[&str]) -> Result<String, HostError> {
        let output = Command::new(tool)
            .args(args)
            // Its words and its fields stay in one form.
            .env("LC_ALL", "C")
            .output()
            .map_err(|source| HostError::Missing { tool, source })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(HostError::Failed {
                command: format!("{tool} {}", args.join(" ")),
                said: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }

    fn nmcli(args: &[&str]) -> Result<String, HostError> {
        Self::run("nmcli", args)
    }

    /// One field of `nmcli -g`, unescaped; `None` if empty or unread.
    fn field(args: &[&str]) -> Option<String> {
        let text = Self::nmcli(args).ok()?;
        let line = text.lines().next()?;
        named(terse_fields(line).first()?)
    }

    fn connection_field(name: &str, field: &str) -> Option<String> {
        Self::field(&["-g", field, "connection", "show", "id", name])
    }
}

impl Host for Nmcli {
    fn route(&self, to: Ipv4Addr) -> Result<Option<Route>, HostError> {
        let to = to.to_string();
        match Self::run("ip", &["-o", "route", "get", &to]) {
            Ok(line) => Ok(parse_route(&line)),
            // The kernel's answer: nowhere to send it.
            Err(HostError::Failed { said, .. }) if said.contains("unreachable") => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn link(&self, interface: &str) -> Link {
        match Self::field(&["-g", "GENERAL.TYPE", "device", "show", interface]).as_deref() {
            None => return Link::Unknown,
            Some("wifi") => {}
            Some(_) => return Link::NotWifi,
        }
        Self::field(&["-g", "GENERAL.CONNECTION", "device", "show", interface])
            .and_then(|name| Self::connection_field(&name, "802-11-wireless.ssid"))
            .map_or(Link::Unknown, Link::Wifi)
    }

    fn radios(&self) -> Result<Vec<Radio>, HostError> {
        let listing = Self::nmcli(&["-t", "-f", "DEVICE,TYPE,CONNECTION", "device"])?;
        Ok(parse_radios(&listing, |name| {
            Self::connection_field(name, "802-11-wireless.mode")
        }))
    }

    fn scan(&self, interface: &str) -> Result<Vec<String>, HostError> {
        let listing = Self::nmcli(&[
            "-t", "-f", "SSID", "device", "wifi", "list", "ifname", interface, "--rescan", "yes",
        ])?;
        Ok(listing
            .lines()
            .filter_map(|line| terse_fields(line).into_iter().next())
            .collect())
    }

    fn join(&self, hop: &Hop, ssid: &str) -> Result<(), HostError> {
        // A profile a killed run left behind is this tool's own.
        let _ = Self::nmcli(&["connection", "delete", "id", PROFILE]);

        let add = profile_args(&hop.interface, ssid);
        let add: Vec<&str> = add.iter().map(String::as_str).collect();
        Self::nmcli(&add)?;
        Self::nmcli(&[
            "--wait",
            UP_WAIT_SECONDS,
            "connection",
            "up",
            "id",
            PROFILE,
            "ifname",
            &hop.interface,
        ])?;
        Ok(())
    }

    fn leave(&self, hop: &Hop) -> Result<(), HostError> {
        let removed = Self::nmcli(&["connection", "delete", "id", PROFILE]);
        if let Some(previous) = &hop.previous {
            Self::nmcli(&[
                "--wait",
                UP_WAIT_SECONDS,
                "connection",
                "up",
                "id",
                previous,
                "ifname",
                &hop.interface,
            ])?;
        }
        removed.map(|_| ())
    }

    fn address_on(&self, ssid: &str) -> Option<Ipv4Addr> {
        let radios = self.radios().ok()?;
        let radio = radios.iter().find(|radio| {
            radio.connection.as_deref().is_some_and(|name| {
                Self::connection_field(name, "802-11-wireless.ssid").as_deref() == Some(ssid)
            })
        })?;
        let address =
            Self::nmcli(&["-g", "IP4.ADDRESS", "device", "show", &radio.interface]).ok()?;
        address.lines().next()?.split('/').next()?.parse().ok()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use std::cell::RefCell;

    /// A computer as a test says it is, recording what was changed on it.
    #[derive(Default)]
    pub(crate) struct FakeHost {
        pub(crate) route: Option<Route>,
        /// `ip` itself cannot run.
        pub(crate) no_route_tool: bool,
        pub(crate) link: Option<Link>,
        pub(crate) radios: Vec<Radio>,
        pub(crate) heard: Vec<String>,
        /// Joining works, and the route then goes to this network.
        pub(crate) joins: bool,
        pub(crate) joined: RefCell<Vec<String>>,
        pub(crate) left: RefCell<Vec<Hop>>,
        pub(crate) on_feeder: RefCell<Option<String>>,
        pub(crate) address: Option<Ipv4Addr>,
    }

    pub(crate) fn wifi_route(interface: &str) -> Option<Route> {
        Some(Route {
            interface: interface.to_string(),
            gateway: None,
            local: false,
        })
    }

    impl Host for FakeHost {
        fn route(&self, _: Ipv4Addr) -> Result<Option<Route>, HostError> {
            if self.no_route_tool {
                return Err(HostError::Missing {
                    tool: "ip",
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                });
            }
            if self.on_feeder.borrow().is_some() {
                return Ok(wifi_route("wlan1"));
            }
            Ok(self.route.clone())
        }

        fn link(&self, _: &str) -> Link {
            if let Some(ssid) = &*self.on_feeder.borrow() {
                return Link::Wifi(ssid.clone());
            }
            self.link.clone().unwrap_or(Link::Unknown)
        }

        fn radios(&self) -> Result<Vec<Radio>, HostError> {
            if self.radios.is_empty() {
                return Err(HostError::Missing {
                    tool: "nmcli",
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                });
            }
            Ok(self.radios.clone())
        }

        fn scan(&self, _: &str) -> Result<Vec<String>, HostError> {
            Ok(self.heard.clone())
        }

        fn join(&self, _: &Hop, ssid: &str) -> Result<(), HostError> {
            self.joined.borrow_mut().push(ssid.to_string());
            if self.joins {
                *self.on_feeder.borrow_mut() = Some(ssid.to_string());
                Ok(())
            } else {
                Err(HostError::Failed {
                    command: "nmcli connection up".to_string(),
                    said: "timed out".to_string(),
                })
            }
        }

        fn leave(&self, hop: &Hop) -> Result<(), HostError> {
            *self.on_feeder.borrow_mut() = None;
            self.left.borrow_mut().push(hop.clone());
            Ok(())
        }

        fn address_on(&self, _: &str) -> Option<Ipv4Addr> {
            self.address
        }
    }

    /// A test holds on to the host a guide owns, to read it after the drop.
    impl Host for std::rc::Rc<FakeHost> {
        fn route(&self, to: Ipv4Addr) -> Result<Option<Route>, HostError> {
            (**self).route(to)
        }

        fn link(&self, interface: &str) -> Link {
            (**self).link(interface)
        }

        fn radios(&self) -> Result<Vec<Radio>, HostError> {
            (**self).radios()
        }

        fn scan(&self, interface: &str) -> Result<Vec<String>, HostError> {
            (**self).scan(interface)
        }

        fn join(&self, hop: &Hop, ssid: &str) -> Result<(), HostError> {
            (**self).join(hop, ssid)
        }

        fn leave(&self, hop: &Hop) -> Result<(), HostError> {
            (**self).leave(hop)
        }

        fn address_on(&self, ssid: &str) -> Option<Ipv4Addr> {
            (**self).address_on(ssid)
        }
    }

    fn host(route: Option<Route>, link: Link) -> FakeHost {
        FakeHost {
            route,
            link: Some(link),
            ..FakeHost::default()
        }
    }

    /// The hazard this module exists for: home Wi-Fi's router is 192.168.1.1
    /// too. Only a route to a network named like the feeder's passes, and each
    /// refusal carries what it found.
    #[test]
    fn only_the_feeder_s_network_passes() {
        let feeder = host(wifi_route("wlan1"), Link::Wifi("PL-AF103-AB12".into()));
        assert_eq!(
            look(&feeder),
            Look::Feeder {
                ssid: "PL-AF103-AB12".into()
            }
        );

        let home = host(wifi_route("wlan0"), Link::Wifi("Maple St".into()));
        let Look::Refused(refused) = look(&home) else {
            panic!("home Wi-Fi is refused");
        };
        assert_eq!(
            refused,
            FrameRefused::OtherNetwork {
                interface: "wlan0".into(),
                ssid: "Maple St".into()
            }
        );
        let words = refused.to_string();
        assert!(
            words.contains("wlan0") && words.contains("Maple St"),
            "{words}"
        );
        assert!(words.contains("nothing was sent"), "{words}");

        assert_eq!(
            look(&host(wifi_route("eth0"), Link::NotWifi)),
            Look::Refused(FrameRefused::NotWifi {
                interface: "eth0".into()
            })
        );
        assert_eq!(
            look(&host(None, Link::Unknown)),
            Look::Refused(FrameRefused::NoRoute)
        );

        // The feeder's network has no gateway, whatever the interface's name.
        let gateway: IpAddr = "10.0.0.1".parse().unwrap();
        let through = host(
            Some(Route {
                interface: "wlan1".into(),
                gateway: Some(gateway),
                local: false,
            }),
            Link::Wifi("PL-AF103-AB12".into()),
        );
        assert_eq!(
            look(&through),
            Look::Refused(FrameRefused::ThroughGateway {
                interface: "wlan1".into(),
                gateway
            })
        );

        let own = host(
            Some(Route {
                interface: "lo".into(),
                gateway: None,
                local: true,
            }),
            Link::Unknown,
        );
        assert_eq!(
            look(&own),
            Look::Refused(FrameRefused::ThisComputer {
                interface: "lo".into()
            })
        );
    }

    /// Where the name cannot be read, it is asked for, never assumed.
    #[test]
    fn an_unreadable_network_is_asked_about() {
        assert_eq!(look(&host(wifi_route("wlan1"), Link::Unknown)), Look::Ask);
        let no_tool = FakeHost {
            no_route_tool: true,
            ..FakeHost::default()
        };
        assert_eq!(look(&no_tool), Look::Ask);

        assert_eq!(confirmed(" PL-AF103-AB12\n").unwrap(), "PL-AF103-AB12");
        assert_eq!(
            confirmed("Maple St").unwrap_err(),
            FrameRefused::NotConfirmed {
                typed: "Maple St".into()
            }
        );
        assert!(confirmed("").is_err());
    }

    fn radio(interface: &str, connection: Option<&str>, hotspot: bool) -> Radio {
        Radio {
            interface: interface.into(),
            connection: connection.map(str::to_string),
            hotspot,
        }
    }

    /// A hop never takes the interface serving a hotspot, and leaves an
    /// interface in use alone if another is free.
    #[test]
    fn a_hop_picks_a_radio_that_carries_no_hotspot() {
        let hotspot = radio("wlan0", Some("Lab"), true);
        let home = radio("wlan1", Some("Home"), false);
        let free = radio("wlan2", None, false);

        assert_eq!(
            pick(&[hotspot.clone(), home.clone(), free.clone()]),
            Some(&free)
        );
        assert_eq!(pick(&[hotspot.clone(), home.clone()]), Some(&home));
        assert_eq!(pick(&[hotspot]), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn feeder_networks_are_found_by_their_prefix() {
        let heard = [
            "Maple St",
            "PL-AF103-AB12",
            "PL-AF103-AB12",
            "",
            "XPL-AF103-1",
        ]
        .map(str::to_string);
        assert_eq!(feeders(&heard), ["PL-AF103-AB12"]);
    }

    /// The physical finding is what makes the join work, so it is pinned, and
    /// the manual steps give the same profile.
    #[test]
    fn the_profile_turns_off_conflict_detection_and_autoconnect() {
        let args = profile_args("wlan1", "PL-AF103-AB12").join(" ");
        assert!(args.contains("ipv4.dad-timeout 0"), "{args}");
        assert!(args.contains("connection.autoconnect no"), "{args}");
        assert!(args.contains("ifname wlan1"), "{args}");
        assert!(args.contains("ssid PL-AF103-AB12"), "{args}");

        let steps = manual_steps(Some("wlan1"), Some("PL-AF103-AB12"));
        assert!(steps.contains(&args), "{steps}");
        assert!(steps.contains("nmcli connection up wellfed-feeder-setup"));
        assert!(steps.contains("nmcli connection delete wellfed-feeder-setup"));

        let unknown = manual_steps(None, None);
        assert!(unknown.contains("PL-AF103-…"), "{unknown}");
        assert!(!unknown.contains("nmcli"), "{unknown}");
    }

    #[test]
    fn route_lines_are_read() {
        assert_eq!(
            parse_route("192.168.1.1 dev wlan1 src 192.168.1.100 uid 1000 \\    cache "),
            wifi_route("wlan1")
        );
        assert_eq!(
            parse_route("192.168.1.1 via 10.0.0.1 dev eth0 src 10.0.0.5 uid 1000"),
            Some(Route {
                interface: "eth0".into(),
                gateway: "10.0.0.1".parse().ok(),
                local: false
            })
        );
        assert!(
            parse_route("local 192.168.1.1 dev lo src 192.168.1.1 uid 1000")
                .unwrap()
                .local
        );
        assert_eq!(parse_route(""), None);
    }

    #[test]
    fn terse_listings_are_read() {
        assert_eq!(
            terse_fields(r"wlan0:wifi:Cafe\: back\\room"),
            ["wlan0", "wifi", r"Cafe: back\room"]
        );

        let listing = "wlan0:wifi:Lab\nwlan1:wifi:--\neth0:ethernet:Wired\nwlan2:wifi:\n";
        let radios = parse_radios(listing, |name| (name == "Lab").then(|| "ap".to_string()));
        assert_eq!(
            radios,
            [
                radio("wlan0", Some("Lab"), true),
                radio("wlan1", None, false),
                radio("wlan2", None, false),
            ]
        );
    }

    #[test]
    fn shell_words_are_quoted_only_when_they_must_be() {
        assert_eq!(quoted("private/review"), "private/review");
        assert_eq!(quoted("Maple St"), "'Maple St'");
        assert_eq!(quoted("it's"), r"'it'\''s'");
        assert_eq!(quoted(""), "''");
    }
}
