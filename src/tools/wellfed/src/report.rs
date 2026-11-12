//! The driver's progress as this command line prints it, for a backup, an
//! update key's enrollment and a restore: each [`Event`] as the line it has
//! always printed, on standard error, so standard output carries only what
//! `--prompts lines` puts there.

use wellfed_client::install::STOCK_VERSION;
use wellfed_client::update_key::Enrolled;
use wellfed_install::Report;
use wellfed_install::event::{Enrollment, Event};

use crate::keys::fingerprint;

/// Prints each event on standard error.
pub(crate) struct Printer;

impl Report for Printer {
    fn event(&self, event: Event) {
        eprintln!("{}", line(&event));
    }
}

/// The words for `event`.
pub(crate) fn line(event: &Event) -> String {
    match event {
        Event::KeyEnrollment { key, outcome } => {
            let id = wellfed_session::hex::Lower(key).to_string();
            match outcome {
                Enrollment::Enrolled(Enrolled::Now) => format!("Enrolled update key {id}."),
                Enrollment::Enrolled(Enrolled::Already) => {
                    format!("The replacement already trusts update key {id}; nothing sent.")
                }
                Enrollment::Unanswered => format!(
                    "No answer came to the enrollment of update key {id}; it is not sent again. \
                     Reading the feeder's signers instead."
                ),
            }
        }
        Event::EnrollUnanswered { error } => format!("No answer: {error}"),
        Event::PressNotAnnounced { error } => {
            format!("Could not ask for the Dispense press: {error}")
        }
        Event::SignersUnread { error } => {
            format!("The feeder's clients could not be read ({error}).")
        }
        Event::RestoreSigned {
            version,
            floor,
            key,
        } => format!(
            "Signed stock {STOCK_VERSION} as version {version} (a downgrade at floor {floor}) \
             with update key {}.",
            wellfed_session::hex::Lower(key)
        ),
        Event::Restoring => {
            "Offering stock: bank 1 gives way, and bank 0 writes stock into bank 1.".to_string()
        }
        Event::RestoreKeyKept { error } => {
            format!("The restore's update key file could not be removed: {error}")
        }
        Event::StaleInstallCancelled => {
            "An earlier install was left pending; cancelling it before this offer.".to_string()
        }
        Event::Floor(floor) => format!("The feeder's floor is version {floor}."),
        Event::BackupFound { path } => {
            format!("Using the stock backup kept at {}.", path.display())
        }
        Event::ReadingBank1 { identity } => format!(
            "Reading bank 1 from feeder {} (about 1.3 MB)...",
            fingerprint(identity)
        ),
        Event::Bank1Read(read) => format!("  {read} bytes read"),
        Event::BackupKept { path } => {
            format!("Kept the verified stock backup at {}.", path.display())
        }
        Event::Written { written, length } => format!("  {written} of {length} bytes written"),
        Event::SessionDropped { error } => {
            format!("  The session dropped ({error}); resuming on a new one.")
        }
        Event::AlreadyActivated => "Bank 1 already runs the build: it was activated.".to_string(),
        Event::Activated => "Activated: the feeder restarts into bank 1.".to_string(),
        // Only the install's run sends these, and only the Android app runs it.
        other @ (Event::AwaitingStock
        | Event::StockServerStarted
        | Event::FrameSent { .. }
        | Event::Waiting(_)
        | Event::VersionLost
        | Event::StockServerEnded { .. }
        | Event::Requesting { .. }
        | Event::StockProgress(_)
        | Event::StockRestarting
        | Event::KeyKnown
        | Event::KeyKnownAt { .. }
        | Event::KnownKeyRefused { .. }
        | Event::Rejoining { .. }
        | Event::Recovered { .. }
        | Event::SetupStarting
        | Event::NoSetupFeeder { .. }
        | Event::SetUp { .. }
        | Event::Joined { .. }
        | Event::ReplacementAnswers { .. }
        | Event::TrialClosed
        | Event::TrialOpen { .. }
        | Event::FeederTried { .. }
        | Event::FloorUnread { .. }
        | Event::Promoting
        | Event::Installed { .. }
        | Event::ResumeRecordKept { .. }) => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bench job's standard error is its evidence, so these words stay as
    /// they are.
    #[test]
    fn a_job_s_output_is_unchanged() {
        assert_eq!(
            line(&Event::Written {
                written: 512,
                length: 1024,
            }),
            "  512 of 1024 bytes written"
        );
        assert_eq!(line(&Event::Floor(7)), "The feeder's floor is version 7.");
    }
}
