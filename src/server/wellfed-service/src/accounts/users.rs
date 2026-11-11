//! The household's accounts, as owners manage them: never leaving the
//! household without an owner, and ending a user's sessions when their
//! password changes or their account is deleted.
//!
//! An owner's account never expires, so the household cannot lose its last
//! owner to time either. A disabled account's sessions are ended when it is
//! extended, so only a new login opens one.

use jiff::Timestamp;
use wellfed_api::{AuditAction, NewUser, RevokeReason, Role, User, UserId, UserPatch};
use wellfed_session::hex;
use wellfed_store::{Change, NewAudit, Sessions, Store, UserUpdate, Whose};

use crate::accounts::sessions::{self, disabled, hash_password, password, user_name};
use crate::accounts::tokens::session_only;
use crate::feeders::phones::{rank, revocation};
use crate::{
    AccountError, Caller, Error, Invalid, Random, Resource, Result, Service, SessionToken,
};

impl Service {
    /// Every account, by name.
    pub async fn users(&self) -> Result<Vec<User>> {
        Ok(self.shared.store.users().await?)
    }

    /// Add an account, as `caller` (an owner) from a login: a token could
    /// otherwise make an owner who logs in and mints tokens of their own.
    pub async fn create_user(&self, caller: &Caller, request: NewUser) -> Result<User> {
        session_only(caller)?;
        let name = user_name(&request.name)?;
        password(&request.password)?;
        known_role(request.role)?;
        owner_never_expires(request.role, request.expires)?;

        let hash = self.hash_password(request.password).await?;
        let user = User {
            id: UserId(self.shared.new_id()?),
            name: name.to_owned(),
            role: request.role,
            expires: request.expires,
        };
        let created = self
            .shared
            .store
            .create_user(wellfed_store::NewUser {
                id: user.id,
                name: &user.name,
                password_hash: &hash,
                role: user.role,
                expires: user.expires,
                created: self.shared.clock.now(),
            })
            .await;
        match created {
            Ok(()) => {}
            Err(wellfed_store::Error::Duplicate(_)) => return Err(AccountError::NameTaken.into()),
            Err(error) => return Err(error.into()),
        }

        let action = AuditAction::UserCreated {
            account: user.id,
            name: user.name.clone(),
            role: user.role,
            expires: user.expires,
        };
        self.record(caller, action).await;
        Ok(user)
    }

    /// Change a user's role, expiry or password, as `actor` (an owner)
    /// on the session `session`. Setting a password, raising a role, and
    /// clearing or extending an expiry take a login: a token could otherwise
    /// set a password and log in as that user, or give an account whose
    /// password its holder knows owner rights or its life back, and so a
    /// login that outlives the token. A token may still demote, shorten an
    /// expiry or delete.
    pub async fn update_user(
        &self,
        actor: &Caller,
        session: Option<&SessionToken>,
        id: UserId,
        patch: UserPatch,
    ) -> Result<User> {
        let caller = actor;
        let actor = &actor.user;
        let current = self.user(id).await?;
        let updated = User {
            role: patch.role.unwrap_or(current.role),
            expires: patch.expires.unwrap_or(current.expires),
            ..current.clone()
        };
        if patch.password.is_some() || grants(&current, &updated) {
            session_only(caller)?;
        }
        known_role(updated.role)?;
        owner_never_expires(updated.role, updated.expires)?;
        let hash = match patch.password {
            Some(new) => {
                password(&new)?;
                Some(self.hash_password(new).await?)
            }
            None => None,
        };

        let now = self.shared.clock.now();
        let enabled = disabled(&current, now) && !disabled(&updated, now);
        let reset = hash.is_some();
        // Whoever held the old password is logged out: everywhere when an
        // owner sets someone else's temporary one, and everywhere but here
        // when they set their own. The sessions end in the change's own
        // transaction, so neither lands without the other.
        let keep = session
            .filter(|_| actor.id == id)
            .and_then(SessionToken::hash);
        let sessions = if enabled || reset {
            Sessions::Ended {
                keep: keep.as_ref(),
            }
        } else {
            Sessions::Kept
        };
        let update = UserUpdate {
            role: updated.role,
            expires: updated.expires,
            password_hash: hash.as_deref(),
            sessions,
        };
        changed(self.shared.store.update_user(id, update).await?)?;
        if let Some(reason) = revocation(&current, &updated) {
            self.revoke_phones(Whose::User(id), reason).await?;
        }

        let (account, name) = (updated.id, updated.name.clone());
        if (updated.role, updated.expires) != (current.role, current.expires) {
            let action = AuditAction::UserChanged {
                account,
                name: name.clone(),
                role: updated.role,
                expires: updated.expires,
            };
            self.record(caller, action).await;
        }
        if reset {
            self.record(caller, AuditAction::PasswordSet { account, name })
                .await;
        }
        Ok(updated)
    }

    /// Delete a user and end their sessions, as `caller` (an owner); their
    /// phones' keys are revoked on every feeder.
    pub async fn delete_user(&self, caller: &Caller, id: UserId) -> Result<()> {
        let user = self.user(id).await?;
        changed(self.shared.store.delete_user(id).await?)?;

        let action = AuditAction::UserDeleted {
            account: id,
            name: user.name,
        };
        self.record(caller, action).await;
        self.revoke_phones(Whose::User(id), RevokeReason::Deleted)
            .await
    }

    async fn user(&self, id: UserId) -> Result<User> {
        let user = self.shared.store.user(id).await?;
        user.ok_or_else(|| Error::NotFound(Resource::User))
    }
}

/// Whether a change from `before` to `after` gives the account more: a
/// higher role, or an expiry cleared or moved later.
fn grants(before: &User, after: &User) -> bool {
    let longer = match (before.expires, after.expires) {
        (Some(_), None) => true,
        (Some(was), Some(now)) => now > was,
        (None, _) => false,
    };
    rank(after.role) > rank(before.role) || longer
}

/// Give a locked-out owner a new temporary password, straight on the
/// database (`wellfed-server reset-owner`), and end their sessions. The
/// owner is `name`, or the only owner when it is `None`. The audit log
/// records it at `now`, as the host's.
pub async fn reset_owner(
    store: &Store,
    name: Option<&str>,
    random: &dyn Random,
    now: Timestamp,
) -> Result<(User, String)> {
    let users = store.users().await?;
    let owner = match name {
        Some(name) => {
            let user = users.into_iter().find(|user| user.name == name.trim());
            let user = user.ok_or(Error::NotFound(Resource::User))?;
            if user.role != Role::Owner {
                return Err(AccountError::NotAnOwner(user.name).into());
            }
            user
        }
        None => {
            let mut owners = users.into_iter().filter(|user| user.role == Role::Owner);
            match (owners.next(), owners.next()) {
                (Some(owner), None) => owner,
                (None, _) => return Err(AccountError::NoOwner.into()),
                (Some(_), Some(_)) => return Err(AccountError::OwnerAmbiguous.into()),
            }
        }
    };

    let mut secret = [0; 10];
    random.fill(&mut secret)?;
    let mut salt = [0; 16];
    random.fill(&mut salt)?;
    let password = hex::Lower(&secret).to_string();
    let typed = password.clone();
    let hash = sessions::blocking(move || hash_password(&typed, &salt)).await?;

    if !store.set_password(owner.id, &hash, None).await? {
        return Err(Error::NotFound(Resource::User));
    }
    let action = AuditAction::PasswordSet {
        account: owner.id,
        name: owner.name.clone(),
    };
    let recorded = store
        .record_audit(NewAudit {
            at: now,
            user: None,
            user_name: None,
            token: None,
            action: &action,
        })
        .await;
    // The password is set: losing it to a failed audit write would leave
    // the owner locked out with no way to learn it.
    if let Err(failure) = recorded {
        tracing::error!(%failure, "the reset was made but not recorded in the audit log");
    }
    Ok((owner, password))
}

/// A role a newer client sent, which this server cannot name, is refused
/// rather than stored: it would grant nothing, and say nothing.
fn known_role(role: Role) -> Result<()> {
    if role == Role::Unknown {
        return Err(Invalid::Role.into());
    }
    Ok(())
}

fn owner_never_expires(role: Role, expires: Option<Timestamp>) -> Result<()> {
    if role == Role::Owner && expires.is_some() {
        return Err(Invalid::OwnerExpires.into());
    }
    Ok(())
}

fn changed(change: Change) -> Result<()> {
    match change {
        Change::Done => Ok(()),
        Change::NotFound => Err(Error::NotFound(Resource::User)),
        Change::LastOwner => Err(AccountError::LastOwner.into()),
    }
}
