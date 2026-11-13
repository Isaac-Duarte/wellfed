//! What a run says to logcat: each step and each ask, and each answer, so
//! that the gaps between the lines show what a stalled run waits for. Read
//! with `adb logcat | grep wellfed_app_bridge`.
//!
//! The owner's passphrase, the setup frame, the code on the screen and any
//! key must never reach this log, where any app with log access or a bug
//! report could read it. So every variant is named here rather than falling
//! to a `Debug` of the whole value: one added later fails this build until
//! someone has decided what of it may be written, and so does a field
//! added to one that is written whole.

use crate::api::install::{InstallAnswer, InstallAsk, InstallEnd, InstallEvent, InstallUpdate};
use crate::api::restore::RestoreEnd;

/// Write `update` to the log, if it says anything.
pub(crate) fn update(update: &InstallUpdate) {
    if let Some(line) = line(update) {
        log::info!("{line}");
    }
}

/// Write the answer to ask `id`.
pub(crate) fn answer(id: u32, answer: &InstallAnswer) {
    log::info!("{}", answered(id, answer));
}

/// Write a restore's end, which reaches its screen past [`update`].
pub(crate) fn restore_end(end: &RestoreEnd) {
    log::info!("{}", restore_ended(end));
}

fn line(update: &InstallUpdate) -> Option<String> {
    match update {
        InstallUpdate::Event { event } => event_line(event),
        InstallUpdate::Ask { id, ask } => Some(format!("ask {id}: {}", asked(ask))),
        InstallUpdate::Ended { end } => Some(ended(end)),
    }
}

fn event_line(event: &InstallEvent) -> Option<String> {
    match event {
        // Many a second, and the steps either side say the same.
        InstallEvent::StockProgress { .. }
        | InstallEvent::BackupRead { .. }
        | InstallEvent::Written { .. } => None,

        InstallEvent::Code { seconds, .. } => {
            Some(format!("a code is on the screen for {seconds} s"))
        }
        InstallEvent::Paired {
            address, rebooting, ..
        } => Some(format!(
            "paired, answering at {address:?}, restarting: {rebooting}"
        )),

        // Status and addresses, with nothing of the owner's in them.
        InstallEvent::StockServerStarted
        | InstallEvent::AwaitingStock
        | InstallEvent::Waiting { why: _ }
        | InstallEvent::VersionLost
        | InstallEvent::StockServerEnded { error: _ }
        | InstallEvent::Requesting { build: _ }
        | InstallEvent::StockRestarting
        | InstallEvent::KeyKnown
        | InstallEvent::SetupStarting
        | InstallEvent::NoSetupFeeder { error: _ }
        | InstallEvent::PressDispense
        | InstallEvent::Joined { address: _ }
        | InstallEvent::ReplacementAnswers { build: _, bank: _ }
        | InstallEvent::ReadingBackup { total: _ }
        | InstallEvent::BackupKept { path: _ }
        | InstallEvent::Promoting
        | InstallEvent::SessionDropped
        | InstallEvent::Activated
        | InstallEvent::Installed { build: _ }
        | InstallEvent::Note { text: _ } => Some(format!("{event:?}")),
    }
}

fn asked(ask: &InstallAsk) -> String {
    match ask {
        // These pieces are the setup exchange, the passphrase among them.
        InstallAsk::WriteLink { pieces } => format!("write {} pieces to the link", pieces.len()),

        InstallAsk::RestartFeeder
        | InstallAsk::JoinFeederNetwork
        | InstallAsk::LeaveFeederNetwork
        | InstallAsk::RecognizeFeeder {
            device: _,
            address: _,
        }
        | InstallAsk::OpenRecoveryWindow
        | InstallAsk::OpenLink
        | InstallAsk::CloseLink
        | InstallAsk::Locate { identity: _ } => format!("{ask:?}"),
    }
}

fn answered(id: u32, answer: &InstallAnswer) -> String {
    match answer {
        InstallAnswer::Done
        | InstallAnswer::Yes { yes: _ }
        | InstallAnswer::Located { address: _ }
        | InstallAnswer::Joined { from: _ }
        | InstallAnswer::Failed { reason: _ } => format!("answer {id}: {answer:?}"),
    }
}

fn ended(end: &InstallEnd) -> String {
    match end {
        InstallEnd::Done
        | InstallEnd::Stopped {
            case: _,
            resume: _,
            detail: _,
        } => format!("ended: {end:?}"),
    }
}

fn restore_ended(end: &RestoreEnd) -> String {
    match end {
        RestoreEnd::Activated
        | RestoreEnd::Uncertain { why: _, detail: _ }
        | RestoreEnd::Stopped { case: _, detail: _ } => format!("restore ended: {end:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The setup exchange goes out as these pieces, and the code stands for
    /// the panel presses: neither may reach the log.
    #[test]
    fn secrets_stay_out_of_the_log() {
        let write = line(&InstallUpdate::Ask {
            id: 7,
            ask: InstallAsk::WriteLink {
                pieces: vec![vec![0xde, 0xad], vec![0xbe, 0xef]],
            },
        })
        .unwrap();
        assert_eq!(write, "ask 7: write 2 pieces to the link");

        let code = line(&InstallUpdate::Event {
            event: InstallEvent::Code {
                code: vec![1, 2, 3, 4],
                seconds: 60,
            },
        })
        .unwrap();
        assert_eq!(code, "a code is on the screen for 60 s");
    }

    /// A wait reads as an ask and, later, its answer.
    #[test]
    fn a_wait_shows_as_an_ask_and_its_answer() {
        let ask = line(&InstallUpdate::Ask {
            id: 3,
            ask: InstallAsk::OpenLink,
        })
        .unwrap();
        assert_eq!(ask, "ask 3: OpenLink");
        assert_eq!(answered(3, &InstallAnswer::Done), "answer 3: Done");
    }

    /// A restore's end is written too, though it never passes an update.
    #[test]
    fn a_restore_s_end_is_written() {
        assert_eq!(
            restore_ended(&RestoreEnd::Activated),
            "restore ended: Activated"
        );
    }

    /// Progress is left out, as it comes many times a second.
    #[test]
    fn progress_stays_out_of_the_log() {
        let written = InstallUpdate::Event {
            event: InstallEvent::Written {
                written: 1,
                length: 2,
            },
        };
        assert_eq!(line(&written), None);
    }
}
