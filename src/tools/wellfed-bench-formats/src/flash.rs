//! What a `wellfed-flash` child does with the feeder's UART, as the rig
//! tags it when it spawns one. The bench's guards on a write read it, and
//! the flash client checks it against its own commands, so it lives
//! outside the server's live state, which the flash client does not link.

use serde::{Deserialize, Serialize};

/// What a `wellfed-flash` child does with the feeder's UART.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashUse {
    /// `read observe`: a receive-only capture.
    Observe,
    /// Anything else that never programs or erases flash: a read, an
    /// inspection, a plan.
    Session,
    /// Programs or erases flash; `counted` when it prints a sector count
    /// as it verifies.
    Write { counted: bool },
}

impl FlashUse {
    /// What the `wellfed-flash` command line `args` does, read from its
    /// subcommand and mode. Only the commands named here as reading are
    /// taken as such: any other, a new or renamed writer among them, is a
    /// write, so the guards on a write never drop away on a rename.
    /// `wellfed-flash` checks this against its own commands.
    pub fn of(args: &[String]) -> Self {
        let words: Vec<&str> = args.iter().take(2).map(String::as_str).collect();
        match words.as_slice() {
            ["read", "observe"] => Self::Observe,
            ["read", _] => Self::Session,
            ["update" | "give-way", "inspect" | "prepare"]
            | ["stock-recovery", "prepare"]
            | [
                "plan-stock-recovery" | "decode-status" | "inspect-journal",
                ..,
            ] => Self::Session,
            ["update", "execute"] => Self::Write { counted: true },
            _ => Self::Write { counted: false },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a command known to leave flash alone reads as one: a writer
    /// renamed or added, or a flag before the subcommand, is a write.
    #[test]
    fn an_unknown_flash_command_is_a_write() {
        let of = |line: &str| FlashUse::of(&line.split(' ').map(String::from).collect::<Vec<_>>());

        assert_eq!(of("update execute"), FlashUse::Write { counted: true });
        for line in [
            "give-way execute",
            "stock-recovery execute",
            "update run",
            "bank0 execute",
            "--root . update execute",
            "",
        ] {
            assert_eq!(of(line), FlashUse::Write { counted: false }, "{line:?}");
        }
        for line in [
            "read full",
            "update inspect",
            "give-way prepare",
            "stock-recovery prepare",
            "inspect-journal j",
        ] {
            assert_eq!(of(line), FlashUse::Session, "{line:?}");
        }
        assert_eq!(of("read observe"), FlashUse::Observe);
    }
}
