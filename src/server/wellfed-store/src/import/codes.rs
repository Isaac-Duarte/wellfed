//! The closed sets an archive carries as text, and the numbers a start
//! reads narrower than SQL stores them, checked before a row goes in. The database takes any text, but its readers refuse a code they do
//! not know, and some of them run at every start (the home) or every login
//! (an account's role): an archive with such a value, hand-edited or from a
//! build with another vocabulary, would commit a server that cannot start.
//! So each is parsed here by the reader's own conversion, and one that
//! does not parse refuses the import, which rolls it all back.

use wellfed_api::{
    ClientRole, CommandKind, CommandState, Hex, InstallState, InstallTarget, RevokeReason, Role,
    Species, TimeFormat, Units,
};
use wellfed_formats::export::Row;

use crate::{Error, Result};

/// Refuse `row` if a code in it is one the store's readers refuse.
pub(super) fn check(row: &Row) -> Result<()> {
    match row {
        Row::Users(user) => known(Role::from_code(&user.role), "account role"),
        Row::Homes(home) => {
            known(Units::from_code(&home.units), "units")?;
            known(TimeFormat::from_code(&home.time_format), "time format")
        }
        Row::Feeders(feeder) => known(ClientRole::from_code(&feeder.role), "feeder role"),
        Row::PhoneKeys(phone) => {
            known(ClientRole::from_code(&phone.role), "phone role")?;
            match &phone.revoke {
                Some(reason) => known(RevokeReason::from_code(reason), "revoke reason"),
                None => Ok(()),
            }
        }
        Row::Commands(command) => {
            known(CommandKind::from_code(&command.kind), "command kind")?;
            known(CommandState::from_code(&command.state), "command state")
        }
        Row::Installs(install) => {
            known(InstallState::from_code(&install.state), "install state")?;
            let name = <[u8; 32]>::try_from(install.release_id.as_slice())
                .map_err(|_| Error::Unknown("install target's name"))?;
            known(
                InstallTarget::from_code(&install.target, Hex(name)),
                "install target",
            )
        }
        Row::Pets(pet) => known(Species::from_code(&pet.species), "species"),
        // Every start that rebuilds the projections reads each event's
        // epoch as the feeder's 32-bit one.
        Row::Events(event) => known(u32::try_from(event.epoch).ok(), "event epoch"),
        _ => Ok(()),
    }
}

fn known<T>(parsed: Option<T>, what: &'static str) -> Result<()> {
    parsed.map(drop).ok_or(Error::Unknown(what))
}
