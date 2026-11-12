//! What the app's screens take from the device API as Dart constants: the
//! feeder's limits, from `wellfed-model`, which the bridge checks every
//! request against, and who may send which op, from the matrix the feeder
//! enforces (`wellfed_proto::access`, through `wellfed_model::Op::allowed`).
//! A control offers only what the feeder takes, and its words name the same
//! numbers, with no copy in Dart to drift.
//!
//! They are generated rather than read through the bridge because a
//! widget runs without the Rust library in the app's widget and golden
//! tests. The files are committed, so the app builds without Rust; this
//! test fails when one is stale, and
//! `UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test protocol`
//! rewrites them.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;

use jiff::Timestamp;
use wellfed_model::{
    Calibration, ClientRole, FeederSettingsPatch, Generation, Op, PLANS, StatsSection, TimeChanges,
    TimeSync, Tune,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Each limit as Dart names it, with its doc comment.
fn limits() -> [(&'static str, &'static str, u32); 10] {
    use wellfed_model::{
        CALL_PLAYS_MAX, CLIENTS_MAX, DESICCANT_DAYS_MAX, DESICCANT_DAYS_MIN, PLANS, PORTION_MG_MAX,
        PORTION_MG_MIN, PORTIONS_MAX, PORTIONS_MIN,
    };

    let count = |n: usize| u32::try_from(n).expect("a count the feeder keeps fits in 32 bits");
    [
        (
            "portionsLeast",
            "The fewest portions one feed or meal asks for.",
            u32::from(PORTIONS_MIN),
        ),
        (
            "portionsMost",
            "The most portions one feed or meal asks for.",
            u32::from(PORTIONS_MAX),
        ),
        ("meals", "The meals a feeder keeps.", count(PLANS)),
        (
            "callPlaysMost",
            "The most times one meal plays the meal call.",
            u32::from(CALL_PLAYS_MAX),
        ),
        (
            "clientsMost",
            "The most keys a feeder lists: owners, members and the server.",
            count(CLIENTS_MAX),
        ),
        (
            "portionMgLeast",
            "The lightest portion weight the feeder stores, in milligrams.",
            PORTION_MG_MIN,
        ),
        (
            "portionMgMost",
            "The heaviest portion weight the feeder stores, in milligrams.",
            PORTION_MG_MAX,
        ),
        (
            "desiccantDaysLeast",
            "The shortest desiccant reminder period the feeder takes, in days.",
            u32::from(DESICCANT_DAYS_MIN),
        ),
        (
            "desiccantDaysMost",
            "The longest desiccant reminder period the feeder takes, in days.",
            u32::from(DESICCANT_DAYS_MAX),
        ),
        (
            "labelBytesMost",
            "The most bytes of a phone's name a feeder keeps, from 1.",
            count(wellfed_proto::setup::LABEL_MAX),
        ),
    ]
}

fn limits_dart() -> String {
    let mut out = String::from(
        "// Generated from wellfed-model by src/app/rust/tests/protocol.rs; do not edit.\n\n\
         /// The feeder's limits: it refuses a value outside them, so a control never\n\
         /// offers one.\n\
         abstract final class Limits {\n",
    );
    for (index, (name, doc, value)) in limits().into_iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        writeln!(out, "  /// {doc}").unwrap();
        writeln!(out, "  static const {name} = {value};").unwrap();
    }
    out.push_str("}\n");
    out
}

/// Compare `want` with the committed file at `relative`, or write it.
fn check(relative: &str, want: &str) {
    let path = root().join(relative);
    if std::env::var_os("UPDATE_BINDINGS").is_some() {
        std::fs::write(&path, want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{} is stale: run UPDATE_BINDINGS=1 cargo test -p wellfed-app-bridge --test protocol",
        path.display()
    );
}

#[test]
fn the_dart_limits_are_the_feeder_s() {
    check("src/app/lib/src/limits.g.dart", &limits_dart());
}

/// Each kind of op the app may send, `name` numbering it: one per variant,
/// and one per statistics section, whose rows differ.
const KINDS: usize = 45;

/// The op's name in Dart. It matches every variant with no wildcard, so an
/// op added to the model fails to compile here until it has a name, and
/// `every_op_has_one_sample` fails until [`ops`] holds one.
fn name(op: &Op) -> &'static str {
    match op {
        Op::Status => "status",
        Op::GetSettings => "getSettings",
        Op::PutSettings { .. } => "putSettings",
        Op::PutWifi { .. } => "putWifi",
        Op::SetLock { .. } => "setLock",
        Op::Play { .. } => "play",
        Op::TimeSync(_) => "timeSync",
        Op::CreateInvite { .. } => "createInvite",
        Op::CreateBoundInvite { .. } => "createBoundInvite",
        Op::ListClients => "listClients",
        Op::RevokeClient { .. } => "revokeClient",
        Op::Feed { .. } => "feed",
        Op::Reboot => "reboot",
        Op::CancelFeed { .. } => "cancelFeed",
        Op::OfferUpdate { .. } => "offerUpdate",
        Op::UpdateChunk { .. } => "updateChunk",
        Op::UpdateState => "updateState",
        Op::CancelUpdate => "cancelUpdate",
        Op::EnrollUpdateKey { .. } => "enrollUpdateKey",
        Op::UpdateKeyStatement { .. } => "updateKeyStatement",
        Op::GetTime => "getTime",
        Op::PutTime { .. } => "putTime",
        Op::TailLog { .. } => "tailLog",
        Op::Stats { section } => match section {
            StatsSection::Counters => "statsCounters",
            StatsSection::Memory => "statsMemory",
            StatsSection::Flash => "statsFlash",
            StatsSection::Power => "statsPower",
            StatsSection::Food => "statsFood",
            StatsSection::Feeds => "statsFeeds",
            StatsSection::Portions => "statsPortions",
        },
        Op::FeedTrace { .. } => "feedTrace",
        Op::GetCalibration => "getCalibration",
        Op::PutCalibration { .. } => "putCalibration",
        Op::ReadBank1 { .. } => "readBank1",
        Op::GetPlans => "getPlans",
        Op::PutPlans { .. } => "putPlans",
        Op::ResetDesiccant => "resetDesiccant",
        Op::GetCall => "getCall",
        Op::StartCall { .. } => "startCall",
        Op::CallChunk { .. } => "callChunk",
        Op::DeleteCall => "deleteCall",
        Op::PlayCall => "playCall",
        Op::SkipMeal { .. } => "skipMeal",
        Op::UnskipMeal { .. } => "unskipMeal",
        Op::GetAlerts => "getAlerts",
    }
}

/// One op of every kind, each with a wire form, so the matrix classes it.
fn ops() -> Vec<Op> {
    let base = Generation(0);
    let mut ops = vec![
        Op::Status,
        Op::GetSettings,
        Op::PutSettings {
            base,
            changes: FeederSettingsPatch::default(),
        },
        Op::PutWifi { base, wifi: None },
        Op::SetLock { locked: true },
        Op::Play { tune: Tune::Chime },
        Op::TimeSync(TimeSync {
            unix_ms: 0,
            offset_s: 0,
            next_change_s: 0,
            next_offset_s: 0,
        }),
        Op::CreateInvite {
            role: ClientRole::Member,
        },
        Op::CreateBoundInvite {
            role: ClientRole::Member,
            key: [0; 32],
        },
        Op::ListClients,
        Op::RevokeClient { key: [0; 32] },
        Op::Feed { portions: 1 },
        Op::Reboot,
        Op::CancelFeed { feed: 0 },
        Op::OfferUpdate {
            manifest: Vec::new(),
            downgrade: None,
        },
        Op::UpdateChunk {
            offset: 0,
            data: Vec::new(),
        },
        Op::UpdateState,
        Op::CancelUpdate,
        Op::EnrollUpdateKey {
            key: [0; 32],
            label: "key".into(),
        },
        Op::UpdateKeyStatement {
            statement: Vec::new(),
        },
        Op::GetTime,
        Op::PutTime {
            base,
            changes: TimeChanges::default(),
        },
        Op::TailLog { after: None },
        Op::FeedTrace { feed: 0, offset: 0 },
        Op::GetCalibration,
        Op::PutCalibration {
            base,
            calibration: Calibration {
                portion_mg: None,
                bowl: wellfed_model::Bowl::Single,
            },
        },
        Op::ReadBank1 { offset: 0 },
        Op::GetPlans,
        Op::PutPlans {
            base,
            plans: [None; PLANS],
        },
        Op::ResetDesiccant,
        Op::GetCall,
        Op::StartCall { bytes: 0, crc: 0 },
        Op::CallChunk {
            offset: 0,
            data: Vec::new(),
        },
        Op::DeleteCall,
        Op::PlayCall,
        Op::SkipMeal {
            slot: 0,
            due: Timestamp::UNIX_EPOCH,
        },
        Op::UnskipMeal {
            slot: 0,
            due: Timestamp::UNIX_EPOCH,
        },
        Op::GetAlerts,
    ];
    ops.extend(StatsSection::ALL.map(|section| Op::Stats { section }));
    ops
}

/// The roles Dart names, in its enum's order.
const ROLES: [(ClientRole, &str); 5] = [
    (ClientRole::Owner, "owner"),
    (ClientRole::Member, "member"),
    (ClientRole::Server, "server"),
    (ClientRole::UpdateSigner, "updateSigner"),
    (ClientRole::Unknown, "unknown"),
];

/// A set of roles as a Dart constant's name: `_ownerMember`, or `_nobody`.
fn set_name(may: &[usize]) -> String {
    let mut name = String::from("_");
    for (index, &role) in may.iter().enumerate() {
        let role = ROLES[role].1;
        if index == 0 {
            name.push_str(role);
        } else {
            let mut chars = role.chars();
            name.extend(chars.next().map(|first| first.to_ascii_uppercase()));
            name.extend(chars);
        }
    }
    if may.is_empty() {
        name.push_str("nobody");
    }
    name
}

fn roles_dart() -> String {
    // Each op with the roles that may send it, as indices into `ROLES`.
    let rows: Vec<(&str, Vec<usize>)> = ops()
        .iter()
        .map(|op| {
            let may = (0..ROLES.len())
                .filter(|&role| op.allowed(ROLES[role].0))
                .collect();
            (name(op), may)
        })
        .collect();
    let sets: BTreeSet<&Vec<usize>> = rows.iter().map(|(_, may)| may).collect();

    let mut out = String::from(
        "// Generated from wellfed_proto::access by src/app/rust/tests/protocol.rs; do not edit.\n\n\
         import 'package:wellfed/src/rust/api/model.dart' show ClientRole;\n\n\
         /// Each op the app may send, and each statistics section.\n\
         enum Ask {\n",
    );
    for (name, _) in &rows {
        writeln!(out, "  {name},").unwrap();
    }
    out.push_str(
        "}\n\n\
         /// Whether a client holding [role] may send [ask], by the feeder's own\n\
         /// rules: its matrix first, then for an invite the role it grants, which\n\
         /// pairing checks next; the invite rows ask about inviting a member. A\n\
         /// role a newer build added may send nothing.\n\
         bool mayAsk(ClientRole role, Ask ask) => _senders(ask).contains(role);\n",
    );
    for may in sets {
        let roles: Vec<String> = may
            .iter()
            .map(|&role| format!("ClientRole.{}", ROLES[role].1))
            .collect();
        let name = set_name(may);
        writeln!(
            out,
            "\nconst Set<ClientRole> {name} = {{{}}};",
            roles.join(", ")
        )
        .unwrap();
    }
    out.push_str("\nSet<ClientRole> _senders(Ask ask) => switch (ask) {\n");
    for (name, may) in &rows {
        writeln!(out, "  Ask.{name} => {},", set_name(may)).unwrap();
    }
    out.push_str("};\n");
    out
}

#[test]
fn every_op_has_one_sample() {
    let ops = ops();
    let names: BTreeSet<&str> = ops.iter().map(name).collect();
    assert_eq!(ops.len(), KINDS, "one sample per kind");
    assert_eq!(names.len(), KINDS, "no kind twice");
    for op in &ops {
        assert!(op.to_proto().is_ok(), "{op:?} has a wire form");
    }
}

#[test]
fn the_dart_roles_are_the_feeder_s() {
    check("src/app/lib/src/roles.g.dart", &roles_dart());
}
