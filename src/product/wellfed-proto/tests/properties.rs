//! Bounded properties of the device protocol's codec: every message a
//! client or device can be handed decodes or is refused without a panic,
//! whatever a decoded message re-encodes to reads back as the same value,
//! and every request a client can build survives the trip to the device.
//! The byte fixtures (`fixtures.rs`) pin the encoding itself; these
//! properties cover the values between them.

use proptest::prelude::*;
use wellfed_proto::api::*;
use wellfed_proto::device::Model;
use wellfed_proto::setup::{self, Answer, Reply, Request};
use wellfed_proto::{Error, decode, decode_padded, encode, trace};

/// A string for a borrowed field. Tests only: each case leaks a few bytes
/// so a generated value can hold `&'static` fields.
fn text(max: usize) -> impl Strategy<Value = &'static str> {
    proptest::collection::vec(any::<char>(), 0..max)
        .prop_map(|chars| &*Box::leak(chars.into_iter().collect::<String>().into_boxed_str()))
}

fn bytes(max: usize) -> impl Strategy<Value = &'static [u8]> {
    proptest::collection::vec(any::<u8>(), 0..max)
        .prop_map(|bytes| &*Box::leak(bytes.into_boxed_slice()))
}

fn wifi() -> impl Strategy<Value = Wifi<'static>> {
    (bytes(40), text(70)).prop_map(|(ssid, psk)| Wifi { ssid, psk })
}

fn role() -> impl Strategy<Value = Role> {
    prop_oneof![
        Just(Role::Owner),
        Just(Role::Member),
        Just(Role::Server),
        Just(Role::UpdateSigner),
    ]
}

fn tune() -> impl Strategy<Value = Tune> {
    prop_oneof![
        Just(Tune::Chime),
        Just(Tune::Meal),
        Just(Tune::Coin),
        Just(Tune::Alert),
        Just(Tune::Test),
        Just(Tune::Twinkle),
    ]
}

fn window() -> impl Strategy<Value = Window> {
    prop_oneof![
        Just(Window::Always),
        (any::<u16>(), any::<u16>()).prop_map(|(start, end)| Window::Between { start, end }),
    ]
}

fn meal_reminder() -> impl Strategy<Value = MealReminder> {
    prop_oneof![
        Just(MealReminder::Off),
        any::<u8>().prop_map(|minutes| MealReminder::Before { minutes }),
    ]
}

fn op() -> impl Strategy<Value = Op<'static>> {
    let changes = (
        any::<Option<bool>>(),
        any::<Option<bool>>(),
        any::<Option<u8>>(),
        proptest::option::of(window()),
        proptest::option::of(window()),
        any::<Option<u8>>(),
        proptest::option::of(meal_reminder()),
    )
        .prop_map(
            |(
                lights,
                sound,
                volume,
                lights_window,
                sound_window,
                desiccant_days,
                meal_reminder,
            )| {
                SettingsChanges {
                    lights,
                    sound,
                    volume,
                    lights_window,
                    sound_window,
                    desiccant_days,
                    meal_reminder,
                }
            },
        );
    let sync = (any::<u64>(), any::<i32>(), any::<u64>(), any::<i32>()).prop_map(
        |(unix_ms, offset_s, next_change_s, next_offset_s)| TimeSync {
            unix_ms,
            offset_s,
            next_change_s,
            next_offset_s,
        },
    );
    prop_oneof![
        Just(Op::Status),
        Just(Op::Feeder(FeederOp::GetSettings)),
        (any::<u32>(), changes)
            .prop_map(|(base, changes)| Op::Feeder(FeederOp::PutSettings { base, changes })),
        (any::<u32>(), proptest::option::of(wifi()))
            .prop_map(|(base, wifi)| Op::PutWifi { base, wifi }),
        any::<bool>().prop_map(|locked| Op::Feeder(FeederOp::SetLock { locked })),
        tune().prop_map(|tune| Op::Feeder(FeederOp::Play { tune })),
        sync.prop_map(Op::TimeSync),
        role().prop_map(|role| Op::CreateInvite { role }),
        (role(), any::<[u8; 32]>()).prop_map(|(role, key)| Op::CreateBoundInvite { role, key }),
        Just(Op::ListClients),
        any::<[u8; 32]>().prop_map(|key| Op::RevokeClient { key }),
        any::<u8>().prop_map(|portions| Op::Feeder(FeederOp::Feed { portions })),
        Just(Op::Reboot),
        any::<u32>().prop_map(|feed| Op::Feeder(FeederOp::CancelFeed { feed })),
        (bytes(200), proptest::option::of(bytes(200))).prop_map(|(manifest, downgrade)| {
            Op::Update(UpdateOp::Offer {
                manifest,
                downgrade,
            })
        }),
        (any::<u32>(), bytes(UPDATE_CHUNK_MAX + 1))
            .prop_map(|(offset, data)| Op::Update(UpdateOp::Chunk { offset, data })),
        Just(Op::Update(UpdateOp::State)),
        Just(Op::Update(UpdateOp::Cancel)),
        Just(Op::Alive),
        (any::<[u8; 32]>(), text(24))
            .prop_map(|(key, label)| Op::Update(UpdateOp::EnrollKey { key, label })),
        bytes(200).prop_map(|statement| Op::Update(UpdateOp::KeyStatement { statement })),
        Just(Op::GetTime),
        (
            any::<u32>(),
            proptest::option::of(text(60)),
            proptest::option::of(ntp())
        )
            .prop_map(|(base, zone, ntp)| Op::PutTime {
                base,
                changes: TimeChanges { zone, ntp },
            }),
        any::<Option<u32>>().prop_map(|after| Op::TailLog { after }),
        (0..StatsSection::ALL.len()).prop_map(|index| Op::Stats {
            section: StatsSection::ALL[index],
        }),
        (any::<u32>(), any::<u16>())
            .prop_map(|(feed, offset)| Op::Feeder(FeederOp::FeedTrace { feed, offset })),
        Just(Op::Feeder(FeederOp::GetPlans)),
        (any::<u32>(), plans())
            .prop_map(|(base, plans)| Op::Feeder(FeederOp::PutPlans { base, plans })),
        Just(Op::Feeder(FeederOp::ResetDesiccant)),
        Just(Op::Feeder(FeederOp::GetCall)),
        (any::<u32>(), any::<u32>())
            .prop_map(|(bytes, crc)| Op::Feeder(FeederOp::StartCall { bytes, crc })),
        (any::<u32>(), bytes(CALL_CHUNK_MAX + 1))
            .prop_map(|(offset, data)| Op::Feeder(FeederOp::CallChunk { offset, data })),
        Just(Op::Feeder(FeederOp::DeleteCall)),
        Just(Op::Feeder(FeederOp::PlayCall)),
        Just(Op::GetAlerts),
        (any::<u8>(), any::<u32>())
            .prop_map(|(slot, due_s)| Op::Feeder(FeederOp::SkipMeal { slot, due_s })),
        (any::<u8>(), any::<u32>())
            .prop_map(|(slot, due_s)| Op::Feeder(FeederOp::UnskipMeal { slot, due_s })),
        (any::<u32>(), any::<u32>())
            .prop_map(|(bytes, crc)| Op::Feeder(FeederOp::DeleteCallIf { bytes, crc })),
    ]
}

fn plans() -> impl Strategy<Value = [Option<Plan>; PLANS_MAX]> {
    let plan = (
        any::<u16>(),
        any::<u8>(),
        any::<u8>(),
        any::<bool>(),
        any::<u8>(),
    )
        .prop_map(|(minute, portions, days, enabled, call)| Plan {
            minute,
            portions,
            days,
            enabled,
            call,
            ext: Tail::EMPTY,
        });
    proptest::array::uniform10(proptest::option::of(plan))
}

/// One trace entry, timed from `at_ms`, at most `step` ms after it.
fn trace_entry(step: u32) -> impl Strategy<Value = (u32, trace::Entry)> {
    use trace::{Entry, Motor};
    let motor = prop_oneof![Just(Motor::Stop), Just(Motor::Drive0), Just(Motor::Drive1)];
    (
        0..step,
        prop_oneof![
            8 => (0u32..5_000).prop_map(|value| Entry::Current { at_ms: 0, value }),
            1 => (1u32..300, any::<bool>()).prop_map(|(edges, high)| Entry::Mark { at_ms: 0, edges, high }),
            1 => (any::<bool>(), any::<bool>())
                .prop_map(|(outlet_clear, food)| Entry::Beams { at_ms: 0, outlet_clear, food }),
            1 => (0u32..6_000).prop_map(|mv| Entry::Battery { at_ms: 0, mv }),
            1 => motor.prop_map(|motor| Entry::Motor { at_ms: 0, motor }),
        ],
    )
}

fn retimed(entry: trace::Entry, at_ms: u32) -> trace::Entry {
    use trace::Entry;
    match entry {
        Entry::Current { value, .. } => Entry::Current { at_ms, value },
        Entry::Mark { edges, high, .. } => Entry::Mark { at_ms, edges, high },
        Entry::Beams {
            outlet_clear, food, ..
        } => Entry::Beams {
            at_ms,
            outlet_clear,
            food,
        },
        Entry::Battery { mv, .. } => Entry::Battery { at_ms, mv },
        Entry::Motor { motor, .. } => Entry::Motor { at_ms, motor },
    }
}

fn ntp() -> impl Strategy<Value = NtpServer> {
    prop_oneof![
        Just(NtpServer::Off),
        any::<u16>().prop_map(|port| NtpServer::Gateway { port }),
        (any::<[u8; 4]>(), any::<u16>()).prop_map(|(ip, port)| NtpServer::Address { ip, port }),
    ]
}

fn client_message() -> impl Strategy<Value = ClientMessage<'static>> {
    prop_oneof![
        (any::<RequestId>(), op()).prop_map(|(id, op)| ClientMessage::Request { id, op }),
        (any::<Option<u64>>(), samples())
            .prop_map(|(after, samples)| ClientMessage::Subscribe { after, samples }),
    ]
}

fn samples() -> impl Strategy<Value = Samples> {
    prop_oneof![Just(Samples::Off), Just(Samples::On)]
}

fn model() -> impl Strategy<Value = Model> {
    proptest::sample::select(Model::ALL.to_vec())
}

fn open() -> impl Strategy<Value = Open> {
    prop_oneof![
        Just(Open::Session),
        any::<u32>().prop_map(|id| Open::Invite { id }),
        Just(Open::Recovery),
    ]
}

fn answer() -> impl Strategy<Value = Answer<'static>> {
    prop_oneof![
        (any::<Option<[u8; 32]>>(), any::<Option<[u8; 32]>>())
            .prop_map(|(open, commit)| Answer::Round { open, commit }),
        (any::<[u8; 32]>(), proptest::option::of(wifi()))
            .prop_map(|(open, wifi)| Answer::Last { open, wifi }),
    ]
}

fn reply() -> impl Strategy<Value = Reply> {
    let refusal = proptest::sample::select(vec![
        setup::Refusal::Busy,
        setup::Refusal::Unconfirmed,
        setup::Refusal::WrongCode,
        setup::Refusal::Full,
        setup::Refusal::NotWritable,
        setup::Refusal::Invalid,
    ]);
    let enrolled = prop_oneof![Just(setup::Enrolled::New), Just(setup::Enrolled::Already)];
    let saved = prop_oneof![
        Just(setup::WifiSaved::Saved),
        Just(setup::WifiSaved::Unchanged)
    ];
    prop_oneof![
        any::<u16>().prop_map(|seconds| Reply::EnterCode { seconds }),
        (any::<Option<[u8; 32]>>(), any::<Option<[u8; 32]>>())
            .prop_map(|(open, commit)| Reply::Round { open, commit }),
        (enrolled, saved).prop_map(|(enrolled, wifi)| Reply::Done { enrolled, wifi }),
        refusal.prop_map(Reply::Refused),
    ]
}

/// Encode `value` and decode it back: the same value, from exactly the
/// encoded bytes. A shorter buffer is refused, never overrun.
macro_rules! round_trip {
    ($ty:ty, $value:expr) => {{
        let value = $value;
        let mut out = [0u8; FRAME_MAX];
        let len = encode(&value, &mut out).unwrap();
        prop_assert_eq!(decode::<$ty>(&out[..len]), Ok(value));
        if len > 0 {
            prop_assert_eq!(
                encode(&value, &mut out[..len - 1]),
                Err(Error::BufferTooSmall)
            );
        }
    }};
}

/// Decode any bytes as `$ty`; whatever decodes re-encodes, and one pass of
/// encoding and decoding is a fixed point, by value and by bytes. The first
/// decode may hold what an encode drops on purpose, a carried tail or a
/// count of skipped alerts ([`a_carried_tail_and_skipped_alerts_are_not_written_back`]),
/// so the pass normalises it: the value it decodes to must then decode
/// back to itself, and encode to the same bytes, which a decoder that lost
/// or invented anything else would not.
macro_rules! stable {
    // A type that carries nothing an encode drops: the first decode is
    // already the fixed point.
    (exact $ty:ty, $bytes:expr) => {{
        stable!($ty, $bytes);
        if let Ok(value) = decode::<$ty>($bytes) {
            let mut out = [0u8; 4 * FRAME_MAX];
            let len = encode(&value, &mut out).unwrap();
            prop_assert_eq!(decode::<$ty>(&out[..len]), Ok(value));
        }
    }};
    ($ty:ty, $bytes:expr) => {{
        if let Ok(value) = decode::<$ty>($bytes) {
            let mut once = [0u8; 4 * FRAME_MAX];
            let len = encode(&value, &mut once).unwrap();
            let normal = decode::<$ty>(&once[..len]);
            prop_assert!(normal.is_ok(), "{:?} does not decode again", value);
            let normal = normal.unwrap();

            let mut twice = [0u8; 4 * FRAME_MAX];
            let again_len = encode(&normal, &mut twice).unwrap();
            prop_assert_eq!(&once[..len], &twice[..again_len]);
            prop_assert_eq!(decode::<$ty>(&twice[..again_len]), Ok(normal));
        }
    }};
}

/// Arbitrary bytes, or a valid variant tag followed by arbitrary bytes, so
/// the cases reach past the first byte.
fn wire() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..64),
        (0u8..16, proptest::collection::vec(any::<u8>(), 0..96)).prop_map(|(tag, mut rest)| {
            rest.insert(0, tag);
            rest
        }),
    ]
}

proptest! {
    #[test]
    fn every_client_message_round_trips(message in client_message()) {
        round_trip!(ClientMessage<'_>, message);
    }

    #[test]
    fn opening_messages_round_trip(
        open in open(),
        major in any::<u16>(),
        model in model(),
        label in text(24),
        role in role(),
        build in text(40),
        epoch in any::<u32>(),
        schema in any::<u32>(),
    ) {
        round_trip!(Open, open);
        round_trip!(Hello, Hello { major });
        round_trip!(Join<'_>, Join { label });
        round_trip!(Joined, Joined::Enrolled { role, model });
        round_trip!(
            Welcome<'_>,
            Welcome::Open { major, model, build, role, epoch, schema }
        );
    }

    #[test]
    fn setup_messages_round_trip(label in text(24), answer in answer(), reply in reply()) {
        round_trip!(Request<'_>, Request { label });
        round_trip!(Answer<'_>, answer);
        let mut out = [0u8; setup::REPLY_MAX];
        let len = encode(&reply, &mut out).unwrap();
        prop_assert_eq!(decode::<Reply>(&out[..len]), Ok(reply));
    }

    /// What the feeder decodes from a peer: the clear opening frame, the
    /// handshake payloads, and each sealed request.
    #[test]
    fn feeder_side_decoders_survive_any_bytes(bytes in wire()) {
        stable!(exact Open, &bytes);
        stable!(exact Hello, &bytes);
        stable!(exact Join<'_>, &bytes);
        // A request can carry a plan whose tail an encode drops.
        stable!(ClientMessage<'_>, &bytes);
        stable!(Request<'_>, &bytes);
        stable!(exact Answer<'_>, &bytes);
    }

    /// What a client decodes from a feeder, or from anything answering on
    /// its address.
    #[test]
    fn client_side_decoders_survive_any_bytes(bytes in wire()) {
        stable!(exact Welcome<'_>, &bytes);
        stable!(exact Joined, &bytes);
        stable!(DeviceMessage<'_>, &bytes);
        stable!(exact Reply, &bytes);

        // A batch's bytes are read only when it is walked: any bytes as a
        // batch walk to an end, and every event they yield re-encodes to
        // the same event, known body or not.
        let mut message = vec![1u8];
        let mut len = bytes.len();
        while len >= 0x80 {
            message.push(u8::try_from(len & 0x7f).unwrap() | 0x80);
            len >>= 7;
        }
        message.push(u8::try_from(len).unwrap());
        message.extend_from_slice(&bytes);
        if let Ok(DeviceMessage::Events(batch)) = decode::<DeviceMessage<'_>>(&message) {
            let mut walked = 0;
            for event in batch.iter() {
                walked += 1;
                let Ok(event) = event else { break };
                let mut out = [0u8; FRAME_MAX];
                let mut writer = EventBatchWriter::new(&mut out);
                writer.push(&event).unwrap();
                let again: Vec<_> = writer.batch().iter().collect();
                prop_assert_eq!(again, vec![Ok(event)]);
            }
            prop_assert!(walked <= bytes.len());
        }
    }

    /// A message is exactly its bytes: one more trailing byte is refused,
    /// and an event body reads back from its zero-padded record field.
    #[test]
    fn trailing_bytes_are_refused(message in client_message(), extra in any::<u8>()) {
        let mut out = [0u8; FRAME_MAX];
        let len = encode(&message, &mut out).unwrap();
        out[len] = extra;
        prop_assert_eq!(decode::<ClientMessage<'_>>(&out[..=len]), Err(Error::Malformed));
    }

    /// A body built as the feeder builds one (each entry pushed, halving
    /// while it is full, ending at the first entry nothing makes room for)
    /// reads back whole: every mark, beam, battery and motor entry pushed
    /// before the end, in order and on time, and the current readings a
    /// subsequence of those pushed.
    #[test]
    fn a_body_reads_back_through_any_halving(
        steps in proptest::collection::vec(trace_entry(400), 0..1_500),
    ) {
        use trace::{Entry, Pushed, Reader, Writer};

        let mut writer = Writer::<{ trace::BODY_MAX }>::new();
        let mut pushed = Vec::new();
        let mut at_ms = 0u32;
        for (step, entry) in steps {
            at_ms += step;
            let entry = retimed(entry, at_ms);
            let kept = loop {
                match writer.push(entry) {
                    Pushed::Kept => break true,
                    Pushed::Full if writer.halve() => {}
                    Pushed::Full => break false,
                }
            };
            if !kept {
                break;
            }
            pushed.push(entry);
        }

        let read: Vec<Entry> = Reader::new(writer.bytes()).map(Result::unwrap).collect();
        let events = |entries: &[Entry]| -> Vec<Entry> {
            entries.iter().copied().filter(|e| !matches!(e, Entry::Current { .. })).collect()
        };
        prop_assert_eq!(events(&read), events(&pushed));
        let mut currents = pushed.iter().filter(|e| matches!(e, Entry::Current { .. }));
        for entry in read.iter().filter(|e| matches!(e, Entry::Current { .. })) {
            prop_assert!(currents.any(|pushed| pushed == entry));
        }
        prop_assert_eq!(
            u32::try_from(read.iter().filter(|e| matches!(e, Entry::Current { .. })).count()).unwrap(),
            writer.currents()
        );
    }

    /// Any bytes read as a body end in entries or one error, never a panic.
    #[test]
    fn any_bytes_read_as_a_body(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let mut reader = trace::Reader::new(&bytes);
        let mut errors = 0;
        for entry in &mut reader {
            errors += usize::from(entry.is_err());
        }
        prop_assert!(errors <= 1);
    }

    #[test]
    fn padded_decoding_survives_any_record(body in any::<[u8; 16]>()) {
        if let Some(event) = decode_padded::<EventBody>(&body) {
            let mut out = [0u8; 16];
            let len = encode(&event, &mut out).unwrap();
            prop_assert_eq!(&out[..len], &body[..len]);
        }
    }
}

/// A body whose variant index is an overlong varint (`0x96 0x00` for 22,
/// `Invited`) decodes as postcard reads it but is no record any build
/// writes: it is refused, and the canonical body still reads.
#[test]
fn a_padded_body_is_read_only_in_its_canonical_bytes() {
    let mut overlong = [0u8; 16];
    overlong[0] = 0x96;
    assert_eq!(decode_padded::<EventBody>(&overlong), None);

    let mut canonical = [0u8; 16];
    canonical[0] = 0x16;
    assert_eq!(
        decode_padded::<EventBody>(&canonical),
        Some(EventBody::Invited {
            client: [0; 7],
            by: [0; 7],
        })
    );
}

/// What an encode drops on purpose: a struct's tail a later build filled
/// is read as carried and written back empty, and a skipped alert is
/// counted on reading and not written. So a newer feeder's answer decodes,
/// and its re-encoding is the fixed point [`stable!`] compares.
#[test]
fn a_carried_tail_and_skipped_alerts_are_not_written_back() {
    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    // `outcome-settings` from `fixtures/v1.txt`, its empty tail (the last
    // byte) replaced by one a newer build filled.
    let pinned =
        hex("00111111111111111111111111111111110a000a0100280001e003b0091501cabe02010a0100");
    let mut newer = pinned[..pinned.len() - 1].to_vec();
    newer.extend_from_slice(&[0x01, 0xff]);
    let Ok(DeviceMessage::Response {
        result: Outcome::Feeder(FeederOutcome::Settings(settings)),
        ..
    }) = decode::<DeviceMessage<'_>>(&newer)
    else {
        panic!("a newer feeder's settings don't decode");
    };
    assert!(!settings.ext.is_empty());
    let mut out = [0u8; FRAME_MAX];
    let message = decode::<DeviceMessage<'_>>(&newer).unwrap();
    let len = encode(&message, &mut out).unwrap();
    assert_eq!(&out[..len], &pinned[..]);

    // A table with an entry this build can't read between two it can.
    #[derive(serde::Serialize)]
    struct Wire<'a> {
        epoch: u32,
        generation: u32,
        more: bool,
        settling: bool,
        alerts: &'a [EncodedAlert<'a>],
    }
    let at = core::num::NonZeroU32::new(5);
    let entry = |alert| EncodedAlert { at, alert };
    let len = encode(
        &Wire {
            epoch: 1,
            generation: 2,
            more: false,
            settling: false,
            alerts: &[entry(&[12]), entry(&[0x70, 0x01]), entry(&[12])],
        },
        &mut out,
    )
    .unwrap();
    let read: AlertsView = decode(&out[..len]).unwrap();
    assert_eq!(read.alerts.skipped, 1);
    let mut again = [0u8; FRAME_MAX];
    let len = encode(&read, &mut again).unwrap();
    let written: AlertsView = decode(&again[..len]).unwrap();
    assert_eq!(written.alerts.skipped, 0);
    assert_eq!(written.alerts.entries, read.alerts.entries);
}
