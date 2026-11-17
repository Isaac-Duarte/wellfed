//! The Wellfed Bench control interface, shared by the board's firmware and
//! the host's `wellfed-relay`.
//!
//! The board answers postcard-rpc requests on a vendor-class USB bulk
//! interface. Both sides build from these types, so a request the board
//! does not know arrives under a key it never matches and is answered
//! `UnknownKey` rather than misread. The rest is pure logic the firmware
//! runs, kept here so it runs in host tests.

#![cfg_attr(not(test), no_std)]

use postcard_rpc::{TopicDirection, endpoints, topics};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// The board's four switched outputs. The discriminant is the channel
/// number the host tools use, so it is fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[repr(u8)]
pub enum Channel {
    /// CH1: feeder USB 5 V (AP2171W enable).
    Power = 1,
    /// CH2: feeder TX held to GND through 4.7 kΩ, the loader-entry strap.
    Strap = 2,
    /// CH3: adapter TXD to feeder RX (74LVC2G125 channel A).
    UartTx = 3,
    /// CH4: feeder TX to adapter RXD (74LVC2G125 channel B).
    UartRx = 4,
}

impl Channel {
    pub const ALL: [Self; 4] = [Self::Power, Self::Strap, Self::UartTx, Self::UartRx];

    /// The channel numbered `n`, or `None` outside 1–4.
    pub fn from_number(n: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| channel.number() == n)
    }

    pub fn number(self) -> u8 {
        self as u8
    }

    /// Bit and array position: CH1 is 0.
    pub fn index(self) -> usize {
        usize::from(self.number() - 1)
    }
}

/// Commanded output state. All off is the de-energised board: feeder
/// unpowered, strap released, UART isolated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Channels([bool; 4]);

impl Channels {
    pub fn get(self, channel: Channel) -> bool {
        self.0[channel.index()]
    }

    pub fn set(&mut self, channel: Channel, on: bool) {
        self.0[channel.index()] = on;
    }

    /// Whether setting `channel` to `on` cuts the feeder's power: CH1 going
    /// from on to off, whoever asks for it. The board counts these
    /// ([`Status::cuts`]), so a host can tell that a power cycle it did not
    /// see ran between two of its requests.
    pub fn cuts_power(self, channel: Channel, on: bool) -> bool {
        channel == Channel::Power && self.get(Channel::Power) && !on
    }
}

/// [`SetChannel`]'s request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Set {
    pub channel: Channel,
    pub on: bool,
}

/// Why the board refused a request that switches an output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Refusal {
    /// A RECOVER sequence holds the outputs for seconds. The request is
    /// refused at once, never queued, so a request the host saw refused
    /// never runs later behind its back.
    Held,
}

/// [`SetChannel`]'s reply: the outputs after the set.
pub type SetReply = Result<Channels, Refusal>;

/// [`Bootsel`]'s reply. The board resets 100 ms after sending it.
pub type BootselReply = Result<(), Refusal>;

/// Which USB-C input the TPS2116 routes to the feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub enum Source {
    Host,
    Aux,
}

/// Latest measurements, already scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Telemetry {
    pub vbus_mv: u32,
    pub current_ma: u32,
    pub source: Source,
    /// The AP2171W's overcurrent flag.
    pub fault: bool,
    /// Feeder TX level at the header, read whether or not CH4 is on.
    pub tx_mv: u32,
}

impl Telemetry {
    pub const fn new() -> Self {
        Self {
            vbus_mv: 0,
            current_ma: 0,
            source: Source::Host,
            fault: false,
            tx_mv: 0,
        }
    }

    /// The feeder's TX idles high whenever its MCU has power. With CH1 off
    /// that means the D-cell batteries are still in.
    pub fn alive(&self) -> bool {
        self.tx_mv >= ALIVE_MV
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

/// [`GetStatus`]'s reply. It reads the last state and never waits, even
/// while RECOVER holds the outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Schema)]
pub struct Status {
    pub channels: Channels,
    /// Feeder power cuts the board has made, counted from a random start at
    /// each board boot. The RECOVER button cycles the feeder's power by
    /// itself, so a host that left the feeder in a session compares the
    /// count to know the power stayed on; the random start keeps a board
    /// reset from looking like no cut.
    pub cuts: u32,
    pub telemetry: Telemetry,
}

endpoints! {
    list = ENDPOINT_LIST;
    | EndpointTy | RequestTy | ResponseTy  | Path             |
    | ---------- | --------- | ----------  | ----             |
    | SetChannel | Set       | SetReply    | "bench/set"      |
    | GetStatus  | ()        | Status      | "bench/status"   |
    | Bootsel    | ()        | BootselReply | "bench/bootsel" |
}

topics! {
    list = TOPICS_IN_LIST;
    direction = TopicDirection::ToServer;
    | TopicTy | MessageTy | Path |
    | ------- | --------- | ---- |
}

topics! {
    list = TOPICS_OUT_LIST;
    direction = TopicDirection::ToClient;
    | TopicTy | MessageTy | Path |
    | ------- | --------- | ---- |
}

/// A RECOVER press, as the panel classifies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Released after at least [`DEBOUNCE_TICKS`] and before
    /// [`LONG_PRESS_TICKS`]: a plain power cycle.
    Short,
    /// Held [`LONG_PRESS_TICKS`]: into the ROM loader, reported once, as it
    /// is reached, with no short press on its release.
    Long,
}

/// Panel ticks (10 ms) a press needs before it counts.
pub const DEBOUNCE_TICKS: u32 = 3;
/// Panel ticks (10 ms) to a long press.
pub const LONG_PRESS_TICKS: u32 = 100;

/// The RECOVER button's press classifier, stepped once per panel tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct PressClassifier {
    held: u32,
    long_sent: bool,
}

impl PressClassifier {
    /// One tick with the button down (`pressed`) or up; the press it
    /// completes, if any.
    pub fn tick(&mut self, pressed: bool) -> Option<Press> {
        if pressed {
            self.held = self.held.saturating_add(1);
            if self.held == LONG_PRESS_TICKS {
                self.long_sent = true;
                return Some(Press::Long);
            }
            return None;
        }

        let press = (self.held >= DEBOUNCE_TICKS && !self.long_sent).then_some(Press::Short);
        *self = Self::default();
        press
    }
}

/// Feeder TX above this counts as driven high by a powered MCU.
pub const ALIVE_MV: u32 = 2000;

/// ADC reference (the 3V3 rail) and full scale of the 12-bit converter.    
const VREF_MV: u32 = 3300;
const ADC_FULL: u32 = 4095;

/// Converts a raw 12-bit ADC reading to millivolts at the pin.
pub fn adc_mv(raw: u16) -> u32 {
    u32::from(raw.min(4095)) * VREF_MV / ADC_FULL
}

/// VBUS_FDR through R12 10k over R13 15k: V = V_pin × 25 / 15.
pub fn vbus_mv(raw: u16) -> u32 {
    adc_mv(raw) * 25 / 15
}

/// INA181A2 (50 V/V) across R23 50 mΩ: 2.5 V per amp, so mA = mV × 0.4.
pub fn current_ma(raw: u16) -> u32 {
    adc_mv(raw) * 2 / 5
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only CH1 going from on to off is a cut: switching it on, setting it
    /// off again, or any other channel is not.
    #[test]
    fn only_switching_the_powered_feeder_off_is_a_cut() {
        let mut channels = Channels::default();
        assert!(!channels.cuts_power(Channel::Power, false));
        assert!(!channels.cuts_power(Channel::Power, true));

        channels.set(Channel::Power, true);
        channels.set(Channel::Strap, true);
        assert!(channels.cuts_power(Channel::Power, false));
        assert!(!channels.cuts_power(Channel::Power, true));
        assert!(!channels.cuts_power(Channel::Strap, false));
        assert!(!channels.cuts_power(Channel::UartTx, false));
    }

    #[test]
    fn all_four_channels_map_in_order() {
        for n in 1..=4u8 {
            let channel = Channel::from_number(n).unwrap();
            assert_eq!(channel.number(), n);
            assert_eq!(channel.index(), usize::from(n - 1));
        }
        assert_eq!(Channel::from_number(0), None);
        assert_eq!(Channel::from_number(5), None);
    }

    /// Each message survives postcard, and the endpoints' request keys
    /// differ, so the board can tell them apart.
    #[test]
    fn messages_round_trip_and_keys_are_distinct() {
        let set = Set {
            channel: Channel::UartRx,
            on: true,
        };
        let back: Set = postcard::from_bytes(&postcard::to_stdvec(&set).unwrap()).unwrap();
        assert_eq!(back, set);

        let mut channels = Channels::default();
        channels.set(Channel::Power, true);
        let status = Status {
            channels,
            cuts: u32::MAX,
            telemetry: Telemetry {
                vbus_mv: 5012,
                current_ma: 123,
                source: Source::Aux,
                fault: true,
                tx_mv: 3290,
            },
        };
        let back: Status = postcard::from_bytes(&postcard::to_stdvec(&status).unwrap()).unwrap();
        assert_eq!(back, status);

        let held: SetReply = Err(Refusal::Held);
        let back: SetReply = postcard::from_bytes(&postcard::to_stdvec(&held).unwrap()).unwrap();
        assert_eq!(back, held);

        let keys: Vec<_> = ENDPOINT_LIST
            .endpoints
            .iter()
            .map(|(_, request, _)| request)
            .collect();
        for (i, key) in keys.iter().enumerate() {
            assert!(!keys[i + 1..].contains(key), "{i}");
        }
    }

    fn presses(down: u32) -> Vec<Press> {
        let mut classifier = PressClassifier::default();
        let mut out: Vec<Press> = (0..down).filter_map(|_| classifier.tick(true)).collect();
        out.extend(classifier.tick(false));
        out
    }

    #[test]
    fn a_bounce_is_no_press_and_a_long_hold_is_one_long_press() {
        assert_eq!(presses(DEBOUNCE_TICKS - 1), []);
        assert_eq!(presses(DEBOUNCE_TICKS), [Press::Short]);
        assert_eq!(presses(LONG_PRESS_TICKS - 1), [Press::Short]);
        assert_eq!(presses(LONG_PRESS_TICKS), [Press::Long]);
        assert_eq!(
            presses(LONG_PRESS_TICKS * 3),
            [Press::Long],
            "no short on release"
        );

        // The next press starts afresh.
        let mut classifier = PressClassifier::default();
        (0..LONG_PRESS_TICKS).for_each(|_| _ = classifier.tick(true));
        assert_eq!(classifier.tick(false), None);
        (0..DEBOUNCE_TICKS).for_each(|_| _ = classifier.tick(true));
        assert_eq!(classifier.tick(false), Some(Press::Short));
    }

    #[test]
    fn scaling_matches_the_dividers() {
        // 5.0 V on VBUS_FDR puts 3.0 V on the pin.
        let raw_3v0 = (3000 * 4095 / 3300) as u16;
        assert!((4990..=5000).contains(&vbus_mv(raw_3v0)));
        // 1 A gives 2.5 V out of the INA181A2.
        let raw_2v5 = (2500 * 4095 / 3300) as u16;
        assert!((995..=1000).contains(&current_ma(raw_2v5)));
        assert_eq!(adc_mv(4095), 3300);
        assert_eq!(adc_mv(u16::MAX), 3300);
        assert_eq!(adc_mv(0), 0);
    }
}
