//! BLE setup, the feeder's side: any length-byte-prefixed messages to a
//! fresh responder, and a genuine client whose third message, codes and
//! Wi-Fi come from the input and whose outcome must follow the stores'
//! rules.
#![no_main]

#[path = "../../product/wellfed-device-api/tests/support/mod.rs"]
mod support;

use wellfed_proto::setup::Wifi;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let messages = support::split(data);
    support::setup_messages(&messages);

    if let [request, codes, ssid, psk, ..] = messages[..] {
        let code = |at: usize| {
            let bytes = codes.get(at..at + 2).unwrap_or(&[0, 0]);
            u16::from_le_bytes([bytes[0], bytes[1]])
        };
        let (feeder, client) = (code(0), code(2));
        let wifi = core::str::from_utf8(psk).ok().map(|psk| Wifi { ssid, psk });

        let outcome = support::setup_client(request, feeder, client, wifi);

        assert_eq!(outcome, support::expected_setup(request, feeder, client, wifi));
    }
});
