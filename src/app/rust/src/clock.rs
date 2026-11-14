//! What this phone keeps about its automatic clock syncs of each feeder
//! ([`AutoSync`]), so that the spacing between them and a refusal hold
//! across its sessions and its catch-ups, and across a process the phone
//! killed in between.
//!
//! The memo lives in `alerts.json`, beside the feeder's watch, once a
//! catch-up has made the feeder's entry there; until then, or with no data
//! directory named (a desktop before its first catch-up, a test), it lives
//! in this process alone. Each store merges with what the file holds, so two
//! processes' memos keep the later sync either sent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use wellfed_client::time::AutoSync;

use crate::alerts;

/// The phone-wide spacing between automatic syncs of one feeder. One that
/// landed leaves an error of about a second, which takes hours to grow
/// back, so this only spaces the syncs a feeder keeps refusing or leaving
/// unanswered: within the clock's 6.6 hours of validity it still leaves
/// several tries.
pub(crate) const SYNC_AGAIN: Duration = Duration::from_secs(30 * 60);

/// The app's data directory, where `alerts.json` is, once Dart names it.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The memos of feeders with no entry in `alerts.json`, by identity.
static MEMORY: Mutex<Option<HashMap<String, AutoSync>>> = Mutex::new(None);

/// Name the app's data directory, once a process.
pub(crate) fn use_dir(dir: PathBuf) {
    // A second naming is the same directory: the first stands.
    let _ = DATA_DIR.set(dir);
}

/// The directory named, if any.
pub(crate) fn dir() -> Option<&'static Path> {
    DATA_DIR.get().map(PathBuf::as_path)
}

fn remembered(identity: &str) -> Option<AutoSync> {
    let memory = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    memory.as_ref()?.get(identity).cloned()
}

fn remember(identity: &str, memo: &AutoSync) {
    let mut memory = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    memory
        .get_or_insert_with(HashMap::new)
        .insert(identity.to_owned(), memo.clone());
}

/// The memo for the feeder `identity` (lowercase hex), from `dir`'s file
/// when it has the feeder, else this process's.
pub(crate) async fn load(dir: Option<&Path>, identity: &str) -> AutoSync {
    if let Some(dir) = dir
        && let Ok(state) = alerts::read(dir).await
        && let Some(memo) = state
            .watched(identity)
            .and_then(|watched| watched.clock.clone())
    {
        return memo;
    }
    remembered(identity).unwrap_or_default()
}

/// Keep `memo` for the feeder `identity`, merged with what was kept.
/// A file that can't be written keeps it in this process alone: a memo
/// only spaces syncs out.
pub(crate) async fn store(dir: Option<&Path>, identity: &str, memo: &AutoSync) {
    let merged = remembered(identity).map_or_else(|| memo.clone(), |old| old.merged(memo));
    remember(identity, &merged);
    if let Some(dir) = dir {
        let _ = alerts::change(dir, |state| {
            if let Some(watched) = state.watched_mut(identity) {
                watched.clock = Some(
                    watched
                        .clock
                        .as_ref()
                        .map_or_else(|| merged.clone(), |old| old.merged(&merged)),
                );
            }
        })
        .await;
    }
}
