//! First-owner setup: a fresh server takes its first owner, or an import,
//! only while it has no account and only for its setup window after the
//! start ([`Timing::setup_window`](crate::Timing::setup_window)), so a server
//! that is reached before its owner sets it up stays open to that for a
//! bounded time. A restart opens the window again.

use wellfed_api::{
    AuditAction, Home, HomeId, Role, Setup, SetupState, TimeFormat, Units, User, UserId,
};
use wellfed_mirror::Zone;
use wellfed_store::NewUser;

use crate::accounts::sessions::{SessionToken, password, user_name};
use crate::{AccountError, Result, Service};

/// The name a setup gives the home it makes.
const HOME_NAME: &str = "Home";

/// Why a fresh server's setup and import are shut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupShut {
    /// An account exists.
    Owned,
    /// No account exists, and the window after the start has passed.
    Expired,
}

impl Service {
    /// Whether a fresh server is waiting for its first owner: no account
    /// exists, and this run started within its setup window.
    pub async fn setup_open(&self) -> Result<bool> {
        Ok(self.setup_shut().await?.is_none())
    }

    /// Why setup and import are shut now; `None` while they are open.
    pub(crate) async fn setup_shut(&self) -> Result<Option<SetupShut>> {
        if self.shared.store.users_exist().await? {
            return Ok(Some(SetupShut::Owned));
        }
        Ok((!self.setup_window_open()).then_some(SetupShut::Expired))
    }

    /// Say in the log, on a server with no account, how long it takes a
    /// setup or an import.
    pub(crate) async fn note_setup_window(&self) -> Result<()> {
        if self.setup_shut().await?.is_none() {
            let window = self.shared.config.timing.setup_window;
            tracing::info!(
                "no account yet: setup and import are open for {} minutes; a restart opens them again",
                window.as_secs() / 60
            );
        }
        Ok(())
    }

    /// Whether this run is still within its setup window, by both the
    /// service's clock and one that never steps: a clock set forward
    /// closes it early, and one set back never keeps it open longer.
    fn setup_window_open(&self) -> bool {
        let window = self.shared.config.timing.setup_window;
        let steady = self.shared.started_steady.elapsed() < window;
        let since = self.shared.clock.now().duration_since(self.shared.started);
        let wall = jiff::SignedDuration::try_from(window).is_ok_and(|window| since < window);
        steady && wall
    }

    /// Whether setup is open, and the host's zone for the page to offer.
    pub async fn setup_state(&self) -> Result<SetupState> {
        Ok(SetupState {
            open: self.setup_open().await?,
            zone: self.shared.config.host_zone.clone(),
        })
    }

    /// Create the first owner and the household in its zone, and log the
    /// owner in. Refused once any user exists, even when two setups race.
    pub async fn setup(&self, setup: Setup) -> Result<(User, SessionToken)> {
        let name = user_name(&setup.name)?;
        password(&setup.password)?;
        let now = self.shared.clock.now();
        let zone = Zone::new(&setup.zone, now)?;
        match self.setup_shut().await? {
            None => {}
            Some(SetupShut::Owned) => return Err(AccountError::SetupClosed.into()),
            Some(SetupShut::Expired) => return Err(AccountError::SetupExpired.into()),
        }

        let hash = self.hash_password(setup.password).await?;
        let _one_at_a_time = self.owner_data.home.lock().await;
        let user = User {
            id: UserId(self.shared.new_id()?),
            name: name.to_owned(),
            role: Role::Owner,
            expires: None,
        };
        let home = Home {
            id: HomeId(self.shared.new_id()?),
            name: HOME_NAME.into(),
            units: Units::Grams,
            time_format: TimeFormat::H24,
            zone: zone.name().to_owned(),
        };
        let created = self
            .shared
            .store
            .create_first_owner(
                NewUser {
                    id: user.id,
                    name: &user.name,
                    password_hash: &hash,
                    role: user.role,
                    expires: None,
                    created: now,
                },
                &home,
            )
            .await?;
        if created.is_none() {
            return Err(AccountError::SetupClosed.into());
        }

        self.mirror.set_zone(zone);
        let token = self.open_session(user.id, &hash).await?;
        let token = token.ok_or(AccountError::BadCredentials)?;
        self.record_as(Some(&user), None, AuditAction::Setup).await;
        Ok((user, token))
    }
}
