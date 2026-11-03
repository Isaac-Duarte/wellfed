//! The writes pinned to a path, kept in the app's private storage beside its
//! own file (`pending.json` next to `wellfed.json`), so the pin outlives the
//! app: a write that may have run is resolved on the path it went on after a
//! restart too. Only the router writes this file. It holds no secrets: a
//! write is kept only when the server route can carry it again under its
//! key (a feed, a cancel, settings, the lock, the desiccant, the plans), and
//! anything else (a Wi-Fi change among them) is kept by kind alone.
//!
//! Written atomically, so a crash keeps the old or the new file. A feed is
//! written down before it is sent, so a phone that stops mid-send finds it
//! pinned at its next start.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wellfed_formats::private;
use wellfed_model::{PLANS, Plan, SettingsPatch};

use crate::api::error::BridgeError;
use crate::api::router::{PendingWrite, RoutePath, WriteKind};

/// A write pinned to the path it went on, from its confirmation to its
/// result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinRecord {
    /// Its one idempotency key, from the owner's confirmation on.
    pub(crate) key: Uuid,
    /// The feeder's identity, lowercase hex.
    pub(crate) feeder: String,
    /// The server's id for the feeder, for a retry under the same key.
    pub(crate) server_feeder: Option<Uuid>,
    pub(crate) path: RoutePath,
    pub(crate) write: PinnedWrite,
    pub(crate) state: PinState,
    /// When the owner confirmed it, by the phone's clock (ms).
    pub(crate) sent_ms: i64,
    /// When it was last tried (ms).
    pub(crate) tried_ms: i64,
    /// A direct feed: the newest event the session knew when it went, so
    /// its `Fed` is one after it; `None` when the session knew none.
    pub(crate) mark: Option<u64>,
    /// Feeds this phone heard answered on the direct path since: none of
    /// their ends is this one's.
    pub(crate) excluded: Vec<u32>,
    /// The owner put it away: it no longer holds a new feed back.
    pub(crate) acknowledged: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PinState {
    /// Written down before it went; nothing has come back yet. A pin
    /// still sending when the app starts was cut off by the app's own
    /// end, and is uncertain.
    Sending,
    /// It may have run.
    Uncertain,
}

/// What a pinned write asked: as much as a retry under its key resends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "write", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PinnedWrite {
    Feed {
        portions: u8,
    },
    Cancel {
        feed: u32,
    },
    Settings {
        base: u32,
        changes: SettingsPatch,
    },
    Lock {
        locked: bool,
    },
    Desiccant,
    Plans {
        base: u32,
        plans: [Option<Plan>; PLANS],
    },
    /// A write the phone sent on its own session that no server route
    /// carries: never sent again, so only its kind is kept.
    Other {
        kind: WriteKind,
    },
}

impl PinnedWrite {
    pub(crate) const fn kind(&self) -> WriteKind {
        match self {
            Self::Feed { .. } => WriteKind::Feed,
            Self::Cancel { .. } => WriteKind::Cancel,
            Self::Settings { .. } => WriteKind::Settings,
            Self::Lock { .. } => WriteKind::Lock,
            Self::Desiccant => WriteKind::Desiccant,
            Self::Plans { .. } => WriteKind::Plans,
            Self::Other { kind } => *kind,
        }
    }
}

impl PinRecord {
    /// As Dart sees it.
    pub(crate) fn view(&self) -> PendingWrite {
        PendingWrite {
            key: self.key.to_string(),
            feeder: self.feeder.clone(),
            path: self.path,
            kind: self.write.kind(),
            portions: match self.write {
                PinnedWrite::Feed { portions } => Some(portions),
                _ => None,
            },
            sent_ms: self.sent_ms,
            acknowledged: self.acknowledged,
        }
    }
}

/// The file's shape.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    pins: Vec<PinRecord>,
}

/// The pins at `path`, none when the router has never written it. A pin the
/// last run left sending is uncertain: that run ended before it heard.
pub(crate) fn load(path: &Path) -> Result<Vec<PinRecord>, BridgeError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file: PinFile = wellfed_formats::load(path)?;
    Ok(file
        .pins
        .into_iter()
        .map(|pin| PinRecord {
            state: PinState::Uncertain,
            ..pin
        })
        .collect())
}

pub(crate) fn save(path: &Path, pins: &[PinRecord]) -> Result<(), BridgeError> {
    let file = PinFile {
        pins: pins.to_vec(),
    };
    private::write_json(path, &file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(key: u128) -> PinRecord {
        PinRecord {
            key: Uuid::from_u128(key),
            feeder: "ab".repeat(32),
            server_feeder: Some(Uuid::from_u128(9)),
            path: RoutePath::Server,
            write: PinnedWrite::Feed { portions: 2 },
            state: PinState::Sending,
            sent_ms: 1_000,
            tried_ms: 1_000,
            mark: None,
            excluded: Vec::new(),
            acknowledged: false,
        }
    }

    #[test]
    fn a_pin_survives_a_restart_and_one_left_sending_comes_back_uncertain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        assert!(load(&path).unwrap().is_empty(), "no file is no pins");

        let settings = PinRecord {
            key: Uuid::from_u128(2),
            path: RoutePath::Direct,
            write: PinnedWrite::Settings {
                base: 4,
                changes: SettingsPatch {
                    volume: Some(30),
                    ..SettingsPatch::default()
                },
            },
            state: PinState::Uncertain,
            mark: Some(17),
            excluded: vec![3],
            acknowledged: true,
            ..feed(2)
        };
        save(&path, &[feed(1), settings.clone()]).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded,
            [
                PinRecord {
                    state: PinState::Uncertain,
                    ..feed(1)
                },
                settings
            ]
        );
    }

    #[test]
    fn a_write_with_no_server_route_keeps_only_its_kind() {
        let wifi = PinRecord {
            write: PinnedWrite::Other {
                kind: WriteKind::Wifi,
            },
            ..feed(3)
        };
        let json = serde_json::to_string(&wifi).unwrap();
        assert!(json.contains("\"kind\":\"wifi\""), "{json}");
        assert_eq!(wifi.view().kind, WriteKind::Wifi);
        assert_eq!(wifi.view().portions, None);
    }
}
