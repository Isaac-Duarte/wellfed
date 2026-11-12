//! Reading the journal: one entry by its key, and pages of it, each the
//! caller's own entries or, for an owner, everyone's.

use uuid::Uuid;
use wellfed_api::{Command, CommandPage, CommandQuery, CommandState, User};
use wellfed_store::{CommandFilter, CommandRow};

use crate::{Error, IdempotencyKey, Invalid, Permission, Resource, Result, Service, can};

const PAGE: u32 = 100;
const PAGE_MAX: u32 = 1000;
/// Past every stored row number (SQLite integers are `i64`).
const CURSOR_MAX: u64 = i64::MAX.unsigned_abs();

impl Service {
    /// One entry: the caller's own, or anyone's for an owner. Another
    /// user's entry is not found, so a member learns nothing of it.
    ///
    /// A client takes a key with no write as never sent, so the key is
    /// retired then: a write under it that reaches the journal later,
    /// after a stall in front of it, is refused rather than run behind the
    /// client's back ([`CommandError::Retired`]).
    pub async fn command(&self, user: &User, key: IdempotencyKey) -> Result<Command> {
        let row = self.shared.store.command(&key).await?;
        match row {
            Some(row) if can(user.role, Permission::AllHistory) || row.user == user.id => {
                Ok(view(row))
            }
            Some(_) => Err(Error::NotFound(Resource::Command)),
            None => {
                let now = self.shared.clock.now();
                if !self.shared.store.retire_key(&key, now).await? {
                    // Journaled in the meantime: read as it now stands.
                    return Box::pin(self.command(user, key)).await;
                }
                Err(Error::NotFound(Resource::Command))
            }
        }
    }

    /// A page of the journal, oldest first: the caller's own entries, or
    /// everyone's for an owner.
    pub async fn commands(&self, user: &User, query: CommandQuery) -> Result<CommandPage> {
        let limit = query.limit.unwrap_or(PAGE);
        if !(1..=PAGE_MAX).contains(&limit) {
            return Err(Invalid::CommandLimit.into());
        }
        let after = query
            .after
            .map(|after| match after.parse::<u64>() {
                Ok(after) if after <= CURSOR_MAX => Ok(after),
                _ => Err(Invalid::CommandCursor),
            })
            .transpose()?;

        // One more than the page shows whether another page follows.
        let mut rows = self
            .shared
            .store
            .commands(CommandFilter {
                user: (!can(user.role, Permission::AllHistory)).then_some(user.id),
                feeder: query.feeder,
                state: query.state.map(CommandState::from),
                after,
                limit: limit + 1,
            })
            .await?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);

        let next = more
            .then(|| rows.last().map(|row| row.seq.to_string()))
            .flatten();
        Ok(CommandPage {
            items: rows.into_iter().map(view).collect(),
            next,
        })
    }
}

pub(crate) fn view(row: CommandRow) -> Command {
    // An unknown entry's outcome only tells this server not to resend it
    // (`retry.rs`): to a client, unknown is unknown, with no outcome.
    let outcome = match row.state {
        CommandState::Unknown => None,
        CommandState::Sent | CommandState::Answered | CommandState::Resolved => row.outcome,
    };
    Command {
        key: Uuid::from_bytes(row.key),
        user: row.user,
        token: row.token,
        feeder: row.feeder,
        kind: row.kind,
        state: row.state,
        outcome,
        feed: row.feed,
        boot_count: row.boot_count,
        sent: row.sent,
        updated: row.updated,
    }
}
