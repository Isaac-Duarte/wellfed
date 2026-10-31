//! The person at the feeder, when a person at a terminal runs the install.
//!
//! This is where the setup frame is guarded. [`Prompter::done`] on
//! [`Prompt::SetupMode`] returns only once this computer is on the feeder's
//! network ([`network::look`]), and otherwise fails, and the run stops with
//! nothing sent. [`Prompter::frame_source`] then reads the route again right
//! before the send, and the socket binds to that address, so a link that dropped
//! fails rather than reaching a router. `--prompts lines` never comes here, and
//! also never sends a frame: `--ssid` is refused there.
//!
//! A network's name proves nothing about whose it is: a neighbour's feeder or a
//! rogue open access point can carry it. So a hop names the network and asks
//! once. That answer is the consent to both the hop and the password going there.
//!
//! The hop itself never touches the frame's no-retry rule: it only moves this
//! computer. A hop is undone at [`Prompt::JoinBack`], or at the latest when the
//! run lets this go, however it ended.

use std::io::Write;
use std::net::Ipv4Addr;
use std::time::Duration;

use wellfed_client::install::PairRoute;
use wellfed_client::setup::panel_position;
use wellfed_formats::bench::prompt::Prompt;
use wellfed_install::{Announcement, PlatformError, Prompter};
use wellfed_session::setup_code::Code;

use super::network::{self, FrameRefused, Hop, Host, Look};
use super::screen::{Screen, Step};
use crate::ask::Asker;

/// How often a hop looks for the feeder's network, which may still be coming
/// up when the person presses Enter.
const SCAN_TRIES: u32 = 6;
const SCAN_PAUSE: Duration = Duration::from_secs(5);

/// The feeder's window for a Dispense press.
const DISPENSE_SECONDS: u16 = 60;

/// What the person is told to do for the Reset hold. The app's words, and a way
/// through for a run that already held it.
const RESET: &str = "On the feeder: if the lock light is on, hold Lock for about 5 s. Then hold \
                     Reset, the rightmost button, for about 5 s.\nThis erases stock's meals, \
                     Wi-Fi and settings. The feeder restarts, says it is ready and opens its own \
                     Wi-Fi network for setup.\nIf that network is already open from an earlier \
                     try, you can skip the hold.\nPress Enter when it is ready.";

/// [`Prompter`] for a person at a terminal.
pub(crate) struct Guide<W, H: Host> {
    asker: Asker<W>,
    screen: Screen,
    host: H,
    /// The hop in force, to undo.
    hop: Option<Hop>,
    /// What a hop found, for the manual steps to name.
    interface: Option<String>,
    ssid: Option<String>,
    /// The interface the person named the network of.
    vouched: Option<String>,
    scan_pause: Duration,
}

impl<W: Write, H: Host> Guide<W, H> {
    pub(crate) const fn new(asker: Asker<W>, screen: Screen, host: H) -> Self {
        Self {
            asker,
            screen,
            host,
            hop: None,
            interface: None,
            ssid: None,
            vouched: None,
            scan_pause: SCAN_PAUSE,
        }
    }

    /// Get this computer onto the feeder's network, and show it is there.
    /// Only `Ok` lets the frame go.
    async fn reach_feeder(&mut self) -> Result<(), PlatformError> {
        if !matches!(network::look(&self.host), Look::Feeder { .. }) {
            self.hop().await?;
        }
        loop {
            match network::look(&self.host) {
                Look::Feeder { ssid, .. } => {
                    self.screen.line(&format!(
                        "This computer is on the feeder's network, {ssid}."
                    ));
                    return Ok(());
                }
                Look::Ask { interface, .. } => {
                    self.screen.line(&self.manual_steps());
                    let typed = self
                        .asker
                        .typed(&format!(
                            "This computer cannot read the name of the network on {interface}. \
                             Your Wi-Fi password goes out over {interface} in the clear, to \
                             whatever answers there, so check first. Look at your Wi-Fi settings, \
                             and type the name of the network {interface} is on now:"
                        ))
                        .await?;
                    match network::confirmed(&typed) {
                        Ok(ssid) => {
                            self.screen
                                .line(&format!("Going by what you typed: {ssid}."));
                            self.vouched = Some(interface);
                            return Ok(());
                        }
                        Err(refused) => self.refused(refused).await?,
                    }
                }
                Look::Refused(refused) => self.refused(refused).await?,
            }
        }
    }

    /// Say what the check found and how to fix it, and let the person check
    /// again. An answer of `q` ends the run with the refusal.
    async fn refused(&mut self, refused: FrameRefused) -> Result<(), PlatformError> {
        self.screen.line(&format!("Refused: {refused}."));
        self.screen.line(&self.manual_steps());
        let answer = self
            .asker
            .typed("Press Enter to check again, or type q to stop.")
            .await?;
        if answer.trim().eq_ignore_ascii_case("q") {
            return Err(Box::new(refused));
        }
        Ok(())
    }

    fn manual_steps(&self) -> String {
        network::manual_steps(self.interface.as_deref(), self.ssid.as_deref())
    }

    /// Move this computer to the feeder's network if it can, on the person's
    /// consent, whose refusal ends the run. Anything else in the way is said
    /// and left to the person: [`Self::reach_feeder`] goes on to check where
    /// this computer is, whatever happened here.
    async fn hop(&mut self) -> Result<(), PlatformError> {
        let radios = match self.host.radios() {
            Ok(radios) => radios,
            Err(error) => {
                self.say_manual(&format!("This computer cannot join by itself ({error})."));
                return Ok(());
            }
        };
        let Some(radio) = network::pick(&radios).cloned() else {
            self.say_manual(
                "Each Wi-Fi interface here serves a hotspot, or could not be read to show it does \
                 not, so this computer will not take one to join the feeder.",
            );
            return Ok(());
        };
        self.interface = Some(radio.interface.clone());

        let found = self.find_feeder(&radio.interface).await;
        let ssid = match found.as_slice() {
            [ssid] => ssid.clone(),
            [] => {
                self.say_manual("The feeder's network did not show up here.");
                return Ok(());
            }
            several => {
                self.say_manual(&format!(
                    "More than one feeder is in setup mode ({}), so choosing is yours.",
                    several.join(", ")
                ));
                return Ok(());
            }
        };
        self.ssid = Some(ssid.clone());

        let drops = radio.connection.as_ref().map_or_else(String::new, |previous| {
            format!(
                "\n{} leaves {previous} for a few seconds, so this computer's connection there, \
                 and any SSH over it, drops until it comes back.",
                radio.interface
            )
        });
        let answer = self
            .asker
            .typed(&format!(
                "Found one feeder network: {ssid}. Check that its name matches your feeder: a \
                 neighbour's feeder, or anyone's open network with that name, would look the \
                 same.{drops}\nIs {ssid} your feeder? Yes joins it and sends it your Wi-Fi name \
                 and password, once and in the clear. [y/N]"
            ))
            .await?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Err(Box::new(FrameRefused::NotConsented { ssid }));
        }

        self.screen
            .line(&format!("Joining {ssid} on {}…", radio.interface));
        let hop = Hop {
            interface: radio.interface,
            previous: radio.connection,
        };
        if let Err(error) = self.host.join(&hop, &ssid) {
            self.say_manual(&format!("Joining {ssid} did not work ({error})."));
            // It may have dropped the connection it was to replace.
            if let Err(error) = self.host.leave(&hop) {
                self.screen
                    .line(&format!("Coming back did not work either ({error})."));
            }
            return Ok(());
        }
        self.hop = Some(hop);
        Ok(())
    }

    fn say_manual(&self, why: &str) {
        self.screen.line(&format!("{why} You can do it by hand."));
    }

    /// The feeder networks `interface` hears, looked for a few times.
    async fn find_feeder(&self, interface: &str) -> Vec<String> {
        for try_number in 1..=SCAN_TRIES {
            if let Ok(heard) = self.host.scan(interface) {
                let found = network::feeders(&heard);
                if !found.is_empty() {
                    return found;
                }
            }
            if try_number < SCAN_TRIES {
                tokio::time::sleep(self.scan_pause).await;
            }
        }
        Vec::new()
    }

    /// Back from the feeder's network: by the hop's undoing, or by the person
    /// if there was none or it did not work.
    async fn come_back(&mut self) -> Result<(), PlatformError> {
        if let Some(hop) = self.hop.take() {
            match self.host.leave(&hop) {
                Ok(()) => {
                    match &hop.previous {
                        Some(previous) => self
                            .screen
                            .line(&format!("This computer is back on {previous}.")),
                        None => self
                            .screen
                            .line("This computer has left the feeder's network."),
                    }
                    return Ok(());
                }
                Err(error) => self
                    .screen
                    .line(&format!("Coming back did not work ({error}).")),
            }
        }
        self.screen.pause();
        Ok(self.asker.done(Prompt::JoinBack).await?)
    }

    fn show_code(&self, code: Code, seconds: u16) {
        let presses = code.presses();
        let positions: Vec<String> = presses
            .iter()
            .map(|&button| panel_position(button).to_string())
            .collect();
        let names: Vec<&str> = presses.iter().map(|button| button.as_str()).collect();
        self.screen.code(
            &format!(
                "PRESS THIS CODE ON THE FEEDER NOW, within {seconds} s, one button at a time:"
            ),
            &positions.join("  "),
            &names.join(", "),
            "Buttons are numbered from the left: 1 Lock, 2 Dispense, 3 Mic, 4 Reset. The code \
             proves this computer is talking to that feeder, and is used once.",
        );
        self.screen.countdown("Key in the code", seconds);
    }
}

/// What a person reads when the code's window closes on `route`.
fn lapsed_words(route: PairRoute) -> &'static str {
    match route {
        PairRoute::Setup => {
            "Time ran out for that code, and nothing was lost: stock is still safe in bank 1, and \
             Wellfed is running from bank 0. The feeder stays in Bluetooth setup, so it needs no \
             restart. Do not press anything until a new code shows.\nPress Enter when you are at \
             the feeder for a new code, or type q to stop."
        }
        PairRoute::Recovery => {
            "Time ran out for that code, and nothing was lost: stock is still safe in bank 1, and \
             Wellfed is running from bank 0. A new code means opening the recovery window again.\n\
             Press Enter to try again, or type q to stop."
        }
    }
}

impl<W: Write, H: Host> Prompter for Guide<W, H> {
    async fn done(&mut self, prompt: Prompt) -> Result<(), PlatformError> {
        match prompt {
            Prompt::SetupMode => {
                self.screen.enter(Step::Reset);
                self.screen.pause();
                self.asker.typed(RESET).await?;
                self.screen.enter(Step::Reach);
                self.screen.line(
                    "This computer joins the feeder's setup network for a few seconds, sends your \
                     Wi-Fi once, then comes back to your network.",
                );
                self.reach_feeder().await
            }
            Prompt::JoinBack => self.come_back().await,
            prompt => {
                if matches!(prompt, Prompt::OpenRecoveryWindow) {
                    self.screen.enter(Step::Claim);
                }
                self.screen.pause();
                Ok(self.asker.done(prompt).await?)
            }
        }
    }

    async fn yes(&mut self, prompt: Prompt) -> Result<bool, PlatformError> {
        if matches!(prompt, Prompt::RecognizeFeeder { .. }) {
            self.screen.enter(Step::Yours);
        }
        self.screen.pause();
        Ok(self.asker.yes(prompt).await?)
    }

    async fn announce(&mut self, announcement: Announcement) -> Result<(), PlatformError> {
        match announcement {
            Announcement::SetupCode { code, seconds } => self.show_code(code, seconds),
            Announcement::PressDispense { key } => {
                self.screen.code(
                    &format!("PRESS DISPENSE ON THE FEEDER NOW, within {DISPENSE_SECONDS} s:"),
                    "2",
                    "Dispense",
                    &format!(
                        "It lets the feeder trust this computer's update key {}, which signs \
                         Wellfed's move to bank 1.",
                        wellfed_formats::hex::encode(&key)
                    ),
                );
                self.screen.countdown("Press Dispense", DISPENSE_SECONDS);
            }
        }
        Ok(())
    }

    async fn settle(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }

    async fn frame_source(&mut self) -> Result<Option<Ipv4Addr>, PlatformError> {
        let source = network::frame_source(&self.host, self.vouched.as_deref())?;
        Ok(Some(source))
    }

    async fn code_lapsed(&mut self, route: PairRoute) -> Result<bool, PlatformError> {
        self.screen.pause();
        let answer = self.asker.typed(lapsed_words(route)).await?;
        Ok(!answer.trim().eq_ignore_ascii_case("q"))
    }
}

/// However the run ended, a hop still in force is undone. Nothing is left to
/// tell about it but a failure.
impl<W, H: Host> Drop for Guide<W, H> {
    fn drop(&mut self) {
        if let Some(hop) = self.hop.take()
            && let Err(error) = self.host.leave(&hop)
        {
            self.screen.line(&format!(
                "This computer did not get back from the feeder's network ({error}). Remove the \
                 profile with: nmcli connection delete {}",
                network::PROFILE
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::network::tests::{FakeHost, SOURCE, wifi_route};
    use super::super::network::{Link, Radio};
    use super::super::screen::tests::{Written, screen};
    use super::super::screen::{Detail, Style};
    use super::*;

    use crate::ask::{AskError, Prompts, scripted, written};

    fn guide(host: FakeHost, lines: &[&str]) -> (Guide<Vec<u8>, FakeHost>, Written) {
        let (screen, shown) = screen(Style::Plain, Detail::Milestones);
        let mut guide = Guide::new(scripted(Prompts::Terminal, lines), screen, host);
        guide.scan_pause = Duration::ZERO;
        (guide, shown)
    }

    fn on_home_wifi() -> FakeHost {
        FakeHost {
            route: wifi_route("wlan0"),
            link: Some(Link::Wifi("Maple St".into())),
            ..FakeHost::default()
        }
    }

    fn home_radio() -> Radio {
        Radio {
            interface: "wlan0".into(),
            connection: Some("Home".into()),
            reserved: false,
        }
    }

    fn refusal(error: &PlatformError) -> Option<&FrameRefused> {
        error.downcast_ref::<FrameRefused>()
    }

    /// The hazard: Enter pressed on home Wi-Fi, with no way to hop. The run's
    /// frame goes only when this returns `Ok`, so nothing goes.
    #[tokio::test]
    async fn on_home_wifi_the_frame_is_refused() {
        // Reset's Enter, then the person looks again, then stops.
        let (mut guide, shown) = guide(on_home_wifi(), &["", "", "q"]);
        let error = guide.done(Prompt::SetupMode).await.unwrap_err();

        assert_eq!(
            refusal(&error),
            Some(&FrameRefused::OtherNetwork {
                interface: "wlan0".into(),
                ssid: "Maple St".into()
            })
        );
        let text = shown.text();
        assert!(text.contains("Step 1 of 5: Reset the feeder"), "{text}");
        assert!(text.contains("nothing was sent"), "{text}");
        assert!(
            text.contains("Join this computer to the feeder's"),
            "{text}"
        );
    }

    /// A closed input is no answer: nothing is assumed, and nothing goes.
    #[tokio::test]
    async fn a_closed_input_sends_nothing() {
        let (mut guide, _) = guide(on_home_wifi(), &[""]);
        let error = guide.done(Prompt::SetupMode).await.unwrap_err();
        assert!(error.downcast_ref::<AskError>().is_some(), "{error}");
    }

    /// Already on the feeder's network: no hop, no question.
    #[tokio::test]
    async fn on_the_feeder_s_network_the_frame_goes() {
        let host = FakeHost {
            route: wifi_route("wlan1"),
            link: Some(Link::Wifi("PL-AF103-AB12".into())),
            ..FakeHost::default()
        };
        let (mut guide, shown) = guide(host, &[""]);
        guide.done(Prompt::SetupMode).await.unwrap();
        assert!(guide.host.joined.borrow().is_empty());
        assert!(shown.text().contains("PL-AF103-AB12"));
    }

    /// Where the name cannot be read, the person names it, and only the
    /// feeder's kind of name lets the frame go.
    #[tokio::test]
    async fn an_unreadable_network_goes_only_on_the_person_s_word() {
        let unknown = || FakeHost {
            route: wifi_route("wlan1"),
            link: Some(Link::Unknown),
            ..FakeHost::default()
        };

        let (mut named, shown) = guide(unknown(), &["", "PL-AF103-AB12"]);
        named.done(Prompt::SetupMode).await.unwrap();
        assert!(shown.text().contains("Going by what you typed"));
        let asked = written(&named.asker).join("\n");
        assert!(asked.contains("the network on wlan1"), "{asked}");
        assert!(asked.contains("over wlan1 in the clear"), "{asked}");

        // The word covers that interface, read again right before the send.
        assert_eq!(named.frame_source().await.unwrap(), Some(SOURCE));
        named.host.route = wifi_route("wlan0");
        let error = named.frame_source().await.unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(&FrameRefused::RouteMoved {
                from: "wlan1".into(),
                to: "wlan0".into()
            })
        );

        let (mut home, _) = guide(unknown(), &["", "Maple St", "q"]);
        let error = home.done(Prompt::SetupMode).await.unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(&FrameRefused::NotConfirmed {
                typed: "Maple St".into()
            })
        );
    }

    /// The hop: warned first, joined, checked, and returned to the connection
    /// it replaced, with its profile gone.
    #[tokio::test]
    async fn a_hop_warns_joins_and_comes_back() {
        let host = FakeHost {
            route: wifi_route("wlan0"),
            link: Some(Link::Wifi("Maple St".into())),
            radios: vec![home_radio()],
            heard: vec!["Maple St".into(), "PL-AF103-AB12".into()],
            joins: true,
            ..FakeHost::default()
        };
        let (mut guide, shown) = guide(host, &["", "y"]);
        guide.done(Prompt::SetupMode).await.unwrap();

        assert_eq!(*guide.host.joined.borrow(), ["PL-AF103-AB12"]);
        let asked = written(&guide.asker).join("\n");
        assert!(asked.contains("leaves Home for a few seconds"), "{asked}");
        assert!(asked.contains("any SSH over it"), "{asked}");
        assert!(asked.contains("Is PL-AF103-AB12 your feeder?"), "{asked}");
        assert!(asked.contains("password"), "{asked}");

        // The route is read again before the send: a dropped link stops it.
        assert_eq!(guide.frame_source().await.unwrap(), Some(SOURCE));
        let hopped = guide.host.on_feeder.borrow_mut().take();
        let error = guide.frame_source().await.unwrap_err();
        assert!(refusal(&error).is_some(), "{error}");
        *guide.host.on_feeder.borrow_mut() = hopped;

        guide.done(Prompt::JoinBack).await.unwrap();
        assert_eq!(
            *guide.host.left.borrow(),
            [Hop {
                interface: "wlan0".into(),
                previous: Some("Home".into())
            }]
        );
        assert!(shown.text().contains("back on Home"));
    }

    /// Only a yes joins and sends. Anything else, Enter included, ends the run
    /// with nothing joined and nothing sent, on a free interface too.
    #[tokio::test]
    async fn a_hop_goes_only_on_a_yes() {
        for answer in ["", "n", "maybe"] {
            let host = FakeHost {
                route: wifi_route("wlan0"),
                link: Some(Link::Wifi("Maple St".into())),
                radios: vec![Radio {
                    interface: "wlan1".into(),
                    connection: None,
                    reserved: false,
                }],
                heard: vec!["PL-AF103-AB12".into()],
                joins: true,
                ..FakeHost::default()
            };
            let (mut guide, _) = guide(host, &["", answer]);
            let error = guide.done(Prompt::SetupMode).await.unwrap_err();
            assert_eq!(
                refusal(&error),
                Some(&FrameRefused::NotConsented {
                    ssid: "PL-AF103-AB12".into()
                })
            );
            assert!(guide.host.joined.borrow().is_empty());
        }
    }

    /// However the run ends after a hop, the hop is undone.
    #[tokio::test]
    async fn a_run_that_ends_after_a_hop_undoes_it() {
        let host = FakeHost {
            route: wifi_route("wlan0"),
            link: Some(Link::Wifi("Maple St".into())),
            radios: vec![home_radio()],
            heard: vec!["PL-AF103-AB12".into()],
            joins: true,
            ..FakeHost::default()
        };
        let host = std::rc::Rc::new(host);
        let (shown, _) = screen(Style::Plain, Detail::Milestones);
        let mut guide = Guide::new(
            scripted(Prompts::Terminal, &["", "y"]),
            shown,
            std::rc::Rc::clone(&host),
        );
        guide.done(Prompt::SetupMode).await.unwrap();
        assert!(host.left.borrow().is_empty());

        drop(guide);
        assert_eq!(host.left.borrow().len(), 1);
    }

    /// A failed join undoes itself and falls back to the person, naming the
    /// network and the exact commands, and still sends nothing.
    #[tokio::test]
    async fn a_failed_join_falls_back_to_exact_commands() {
        let host = FakeHost {
            route: wifi_route("wlan0"),
            link: Some(Link::Wifi("Maple St".into())),
            radios: vec![home_radio()],
            heard: vec!["PL-AF103-AB12".into()],
            joins: false,
            ..FakeHost::default()
        };
        let (mut guide, shown) = guide(host, &["", "y", "q"]);
        let error = guide.done(Prompt::SetupMode).await.unwrap_err();

        assert!(refusal(&error).is_some(), "{error}");
        assert_eq!(guide.host.left.borrow().len(), 1);
        let text = shown.text();
        assert!(text.contains("did not work (nmcli"), "{text}");
        assert!(text.contains("ipv4.dad-timeout 0"), "{text}");
        assert!(text.contains("ssid PL-AF103-AB12"), "{text}");
    }

    /// No hotspot's interface is taken, and no nmcli is no problem but the
    /// person's: both reach the same check.
    #[tokio::test]
    async fn no_radio_or_no_nmcli_falls_back_to_the_person() {
        let hotspot_only = FakeHost {
            route: wifi_route("wlan0"),
            link: Some(Link::Wifi("Lab".into())),
            radios: vec![Radio {
                interface: "wlan0".into(),
                connection: Some("Lab".into()),
                reserved: true,
            }],
            ..FakeHost::default()
        };
        let (mut guide_one, shown) = guide(hotspot_only, &["", "q"]);
        assert!(guide_one.done(Prompt::SetupMode).await.is_err());
        assert!(guide_one.host.joined.borrow().is_empty());
        assert!(shown.text().contains("will not take one"));
        // The steps name the interface and network as placeholders here.
        assert!(shown.text().contains("ifname <wifi-interface>"));

        let (mut no_nmcli, shown) = guide(on_home_wifi(), &["", "q"]);
        assert!(no_nmcli.done(Prompt::SetupMode).await.is_err());
        assert!(shown.text().contains("cannot join by itself"));
        assert!(shown.text().contains("ssid <feeder-network>"));
    }

    /// Two feeders in setup mode: which one is the person's to say.
    #[tokio::test]
    async fn two_feeders_are_not_chosen_between() {
        let host = FakeHost {
            route: wifi_route("wlan0"),
            link: Some(Link::Wifi("Maple St".into())),
            radios: vec![home_radio()],
            heard: vec!["PL-AF103-AAAA".into(), "PL-AF103-BBBB".into()],
            joins: true,
            ..FakeHost::default()
        };
        let (mut guide, shown) = guide(host, &["", "q"]);
        assert!(guide.done(Prompt::SetupMode).await.is_err());
        assert!(guide.host.joined.borrow().is_empty());
        let text = shown.text();
        assert!(text.contains("PL-AF103-AAAA, PL-AF103-BBBB"), "{text}");
        assert!(text.contains("ifname wlan0"), "{text}");
        assert!(text.contains("ssid <feeder-network>"), "{text}");
    }

    #[tokio::test]
    async fn a_lapsed_code_says_nothing_was_lost_and_offers_a_new_one() {
        let (mut guide, _) = guide(FakeHost::default(), &["", "q"]);
        assert!(guide.code_lapsed(PairRoute::Setup).await.unwrap());
        assert!(!guide.code_lapsed(PairRoute::Setup).await.unwrap());

        let asked = written(&guide.asker).join("\n");
        assert!(asked.contains("nothing was lost"), "{asked}");
        assert!(asked.contains("safe in bank 1"), "{asked}");
        assert!(asked.contains("needs no restart"), "{asked}");
    }

    /// The code stands out and reads nothing: the run goes straight on to the
    /// feeder's window.
    #[tokio::test]
    async fn the_code_shows_without_waiting() {
        use wellfed_session::setup_code::Code;

        let (mut guide, shown) = guide(FakeHost::default(), &[]);
        guide
            .announce(Announcement::SetupCode {
                code: Code::from_bits(0),
                seconds: 60,
            })
            .await
            .unwrap();
        let text = shown.text();
        assert!(text.contains("PRESS THIS CODE ON THE FEEDER NOW"), "{text}");
        assert!(text.contains("within 60 s"), "{text}");
        assert!(
            text.contains("1 Lock, 2 Dispense, 3 Mic, 4 Reset"),
            "{text}"
        );
    }
}
