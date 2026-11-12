//! The household: its name, how it shows weights and times, and its zone.
//!
//! The zone is the feeders' too. It is stored as its canonical IANA name;
//! the mirror sets each feeder's clock in it and gives each feeder its
//! POSIX rule, so a feeder keeps local time across DST with the server
//! down. A zone no rule describes is refused, since a feeder could not keep
//! it.

use tracing::error;
use wellfed_api::{AuditAction, Home, HomePatch, Units};
use wellfed_mirror::Zone;

use crate::names::bounded;
use crate::{Caller, Error, Invalid, Resource, Result, Service};

impl Service {
    pub async fn home(&self) -> Result<Home> {
        let home = self.shared.store.home().await?;
        home.ok_or(Error::NotFound(Resource::Home))
    }

    /// Change the household, as `caller` (an owner). A new zone goes to
    /// every feeder: the online ones now, the others when they reconnect.
    pub async fn update_home(&self, caller: &Caller, patch: HomePatch) -> Result<Home> {
        let _one_at_a_time = self.owner_data.home.lock().await;
        let mut home = self.home().await?;
        if let Some(name) = patch.name {
            home.name = home_name(&name)?.to_owned();
        }
        if let Some(units) = patch.units {
            // Units a newer client sent, which this server cannot name.
            if units == Units::Unknown {
                return Err(Invalid::Units.into());
            }
            home.units = units;
        }
        if let Some(format) = patch.time_format {
            home.time_format = format;
        }
        let zone = patch
            .zone
            .map(|name| Zone::new(&name, self.shared.clock.now()))
            .transpose()?;
        if let Some(zone) = &zone {
            home.zone = zone.name().to_owned();
        }

        self.shared.store.set_home(&home).await?;
        if let Some(zone) = zone {
            self.mirror.set_zone(zone);
        }
        let action = AuditAction::HomeChanged { home: home.clone() };
        self.record(caller, action).await;
        Ok(home)
    }

    /// Give the mirror the stored zone, at start. A zone the tzdb no
    /// longer knows leaves the feeders' clocks in UTC until an owner picks
    /// another; it is logged rather than stopping the server.
    pub(crate) async fn load_zone(&self) -> Result<()> {
        let Some(home) = self.shared.store.home().await? else {
            return Ok(());
        };
        match Zone::stored(&home.zone, self.shared.clock.now()) {
            Ok(zone) => self.mirror.set_zone(zone),
            Err(refused) => error!("the home's zone: {refused}; feeders' clocks stay in UTC"),
        }
        Ok(())
    }
}

fn home_name(name: &str) -> Result<&str> {
    Ok(bounded(name, Invalid::HomeName)?)
}
