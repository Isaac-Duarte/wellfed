//! `install`: an owner's install of a kept release on one feeder, followed
//! on the stream to its end; without a release, where the feeder's running
//! or last install stands.
//!
//! An install is not a keyed write: the server runs one at a time and
//! refuses another while it runs, and the feeder reports what it already
//! runs, so nothing installs twice. A failed install exits 1 with the
//! server's reason and is not started again. One whose offer may have
//! taken (the feeder went silent after it, or the server stopped while it
//! ran), and one this build cannot read the end of, exits 3.

use anyhow::{Result, bail};
use clap::Args;
use wellfed_server_client::api::{
    Hex, Install, InstallFailure, InstallState, StreamEvent, StreamQuery,
};
use wellfed_server_client::{Client, Updates};

use super::feeders::Which;
use super::{Output, Settled, text};

#[derive(Args)]
pub(crate) struct InstallArgs {
    #[command(flatten)]
    which: Which,
    /// The release's id, as `releases list` shows it. Without one, the
    /// feeder's running or last install.
    #[arg(value_parser = release_id)]
    release: Option<Hex<32>>,
}

pub(super) async fn install(client: &Client, args: InstallArgs, out: Output) -> Result<Settled> {
    let feeder = args.which.id(client).await?;
    let Some(release) = args.release else {
        let install = client.install_state(feeder).await?;
        out.show(&install, text::install)?;
        return Ok(Settled::Known);
    };

    // Opened first, so the end cannot slip past.
    let mut updates = client.stream(&StreamQuery::default(), None).await?;
    let started = client.install(feeder, release).await?;
    out.show(&started, text::install)?;

    let ended = match follow(&mut updates, &started, out).await? {
        Some(ended) => ended,
        // The stream ended first: read where it stands.
        None => client.install_state(feeder).await?,
    };
    if ended.id != started.id || ended.state == InstallState::Running {
        out.show(&ended, text::install)?;
    }
    match settled(&started, &ended) {
        Ended::Done => Ok(Settled::Known),
        Ended::Failed(failure) => bail!("the install failed: {}", text::word(&failure)),
        Ended::Running => {
            eprintln!(
                "The stream ended while the install ran; nothing is sent again. `install` \
                 without a release reads where it stands."
            );
            Ok(Settled::Unknown)
        }
        Ended::Another => {
            eprintln!(
                "The stream ended, and the feeder's latest install is another one, so this \
                 one's end was not read; nothing is sent again. `install` without a release \
                 reads where the feeder stands."
            );
            Ok(Settled::Unknown)
        }
        Ended::Uncertain(why) => {
            eprintln!(
                "Uncertain: {}. The feeder may run the release already, or still its own build; \
                 nothing is sent again. `install` without a release, or the feeder's status, \
                 tells.",
                uncertain_words(why)
            );
            Ok(Settled::Unknown)
        }
    }
}

/// How an install the server ran ended, as this command reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ended {
    /// Bank 1 is activated, or already ran the build.
    Done,
    /// It ended before anything could take: the feeder keeps its build.
    Failed(InstallFailure),
    /// The last word the command has says it still runs.
    Running,
    /// The last word the command has is another install's.
    Another,
    /// The offer may have taken, or the end is one this build cannot read.
    Uncertain(Option<InstallFailure>),
}

/// `ended`, the last word on `started`. Only a failure that says the offer
/// did not take is definite: a feeder gone silent after it, a server that
/// stopped while it ran or failed on its own, and an end a newer server
/// added are not.
pub(super) fn settled(started: &Install, ended: &Install) -> Ended {
    if ended.id != started.id {
        return Ended::Another;
    }
    match (ended.state, ended.failure) {
        (InstallState::Activated | InstallState::AlreadyRunning, _) => Ended::Done,
        (InstallState::Running, _) => Ended::Running,
        (InstallState::Failed, Some(failure)) if !took(failure) => Ended::Failed(failure),
        (InstallState::Failed | InstallState::Unknown, failure) => Ended::Uncertain(failure),
    }
}

/// Whether the offer may have taken all the same when an install ended
/// in `failure`. No wildcard, so a new failure cannot land without its
/// decision.
fn took(failure: InstallFailure) -> bool {
    match failure {
        InstallFailure::UpdateRefused { .. }
        | InstallFailure::Refused { .. }
        | InstallFailure::Stopped { .. }
        | InstallFailure::Lost
        | InstallFailure::Unreachable { offered: false } => false,
        // An older server recorded an answer it could not read after the
        // offer as `unexpected`, so one may follow an offer that took.
        InstallFailure::Unreachable { offered: true }
        | InstallFailure::Unexpected
        | InstallFailure::Server
        | InstallFailure::Interrupted
        | InstallFailure::Unknown => true,
    }
}

/// Why an install's end is uncertain, as one clause.
pub(super) fn uncertain_words(why: Option<InstallFailure>) -> String {
    match why {
        Some(InstallFailure::Unreachable { offered: true }) => {
            "the feeder stopped answering, or answered in a way the server cannot read, after \
             the offer went out"
                .into()
        }
        Some(InstallFailure::Interrupted) => {
            "the server stopped while the install ran, perhaps after the offer went out".into()
        }
        Some(InstallFailure::Unknown) | None => "it ended in a way this build cannot read".into(),
        Some(failure) => format!("it ended {} after the offer went out", text::word(&failure)),
    }
}

/// Follow `started` on the stream, showing each change, until it ends;
/// `None` if the stream ends first.
pub(super) async fn follow(
    updates: &mut Updates,
    started: &Install,
    out: Output,
) -> Result<Option<Install>> {
    while let Some(update) = updates.next().await {
        let Ok(update) = update else {
            return Ok(None);
        };
        let StreamEvent::Install { install, .. } = update.event else {
            continue;
        };
        if install.id != started.id || install == *started {
            continue;
        }
        out.show(&install, text::install)?;
        if install.state != InstallState::Running {
            return Ok(Some(install));
        }
    }
    Ok(None)
}

/// A release id as the API spells it: 64 lowercase hex digits.
fn release_id(text: &str) -> Result<Hex<32>, NotReleaseId> {
    wellfed_session::hex::decode_array(text.as_bytes())
        .map(Hex)
        .ok_or(NotReleaseId)
}

#[derive(Debug, thiserror::Error)]
#[error("a release id is 64 lowercase hex digits, as `releases list` shows it")]
struct NotReleaseId;

#[cfg(test)]
mod tests {
    use uuid::Uuid;
    use wellfed_model::UpdateRefusal;
    use wellfed_server_client::api::{FeederId, InstallId, InstallTarget, UserId};

    use super::*;

    /// Each end, as the command reads it: a failure exits 1 only when the
    /// offer surely did not take, and an end this build cannot read, or a
    /// newer server's state, is uncertain rather than a success.
    #[test]
    fn an_install_ends_definite_only_when_nothing_took() {
        let started = install(InstallState::Running, None);
        let ended = |state, failure| settled(&started, &install(state, failure));

        assert_eq!(ended(InstallState::Activated, None), Ended::Done);
        assert_eq!(ended(InstallState::AlreadyRunning, None), Ended::Done);
        assert_eq!(ended(InstallState::Running, None), Ended::Running);
        for definite in [
            InstallFailure::UpdateRefused {
                refusal: UpdateRefusal::Key,
            },
            InstallFailure::Lost,
            InstallFailure::Unreachable { offered: false },
        ] {
            assert_eq!(
                ended(InstallState::Failed, Some(definite)),
                Ended::Failed(definite)
            );
        }
        for uncertain in [
            InstallFailure::Unreachable { offered: true },
            InstallFailure::Interrupted,
            InstallFailure::Unexpected,
            InstallFailure::Server,
            InstallFailure::Unknown,
        ] {
            assert_eq!(
                ended(InstallState::Failed, Some(uncertain)),
                Ended::Uncertain(Some(uncertain))
            );
        }
        assert_eq!(ended(InstallState::Failed, None), Ended::Uncertain(None));
        assert_eq!(ended(InstallState::Unknown, None), Ended::Uncertain(None));

        // Another install's row says nothing of this one.
        let mut other = install(InstallState::Failed, Some(InstallFailure::Lost));
        other.id = InstallId(Uuid::from_u128(2));
        assert_eq!(settled(&started, &other), Ended::Another);
    }

    fn install(state: InstallState, failure: Option<InstallFailure>) -> Install {
        Install {
            id: InstallId(Uuid::from_u128(1)),
            feeder: FeederId(Uuid::nil()),
            target: InstallTarget::Unknown,
            version: 7,
            by: UserId(Uuid::nil()),
            started: jiff::Timestamp::UNIX_EPOCH,
            state,
            written: 0,
            image_len: 0,
            failure,
            finished: None,
        }
    }

    #[test]
    fn a_release_id_is_its_lowercase_hex() {
        let id = "ab".repeat(32);
        assert_eq!(release_id(&id).ok(), Some(Hex([0xab; 32])));
        assert!(release_id(&"AB".repeat(32)).is_err());
        assert!(release_id("ab").is_err());
    }
}
