//! Users, the home they share (written with the first owner), and their
//! browser sessions.

use jiff::Timestamp;
use uuid::Uuid;
use wellfed_api::{Home, HomeId, Role, User, UserId};

use crate::convert::{millis, timestamp};
use crate::homes::insert_home;
use crate::{Error, Result, Store};

mod sessions;

pub struct NewUser<'a> {
    pub id: UserId,
    pub name: &'a str,
    pub password_hash: &'a str,
    pub role: Role,
    pub expires: Option<Timestamp>,
    pub created: Timestamp,
}

/// A user and the hash their password is checked against.
pub struct Login {
    pub user: User,
    pub password_hash: String,
}

/// A session and the user it belongs to.
pub struct SessionRow {
    pub user: User,
    pub last_used: Timestamp,
}

/// What became of a change to an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Done,
    NotFound,
    /// Refused: no owner would be left.
    LastOwner,
}

impl Store {
    pub async fn users_exist(&self) -> Result<bool> {
        let row = sqlx::query!(r#"SELECT EXISTS (SELECT 1 FROM users) AS "exists!: bool""#)
            .fetch_one(&self.reader)
            .await?;
        Ok(row.exists)
    }

    /// Create `user` and the home only while no user exists, in one
    /// transaction, so two racing setups cannot both win. The home's stored
    /// id once created, `None` when an owner already exists.
    pub async fn create_first_owner(
        &self,
        user: NewUser<'_>,
        home: &Home,
    ) -> Result<Option<HomeId>> {
        let (created, expires) = (millis(user.created), user.expires.map(millis));
        let role = user.role.code();

        let mut tx = self.writer.begin().await?;
        let done = sqlx::query!(
            "INSERT INTO users (id, name, password_hash, role, expires_ms, created_ms)
             SELECT ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users)",
            user.id.0,
            user.name,
            user.password_hash,
            role,
            expires,
            created,
        )
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() != 1 {
            return Ok(None);
        }
        let stored = insert_home(&mut *tx, home, user.created).await?;
        tx.commit().await?;
        Ok(Some(stored))
    }

    pub async fn login(&self, name: &str) -> Result<Option<Login>> {
        let row = sqlx::query!(
            r#"SELECT id AS "id: Uuid", name, password_hash, role, expires_ms
               FROM users WHERE name = ?"#,
            name
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(|row| {
            Ok(Login {
                user: user(row.id, row.name, &row.role, row.expires_ms)?,
                password_hash: row.password_hash,
            })
        })
        .transpose()
    }

    /// Every user, by name.
    pub async fn users(&self) -> Result<Vec<User>> {
        let rows = sqlx::query!(
            r#"SELECT id AS "id: Uuid", name, role, expires_ms FROM users ORDER BY name"#
        )
        .fetch_all(&self.reader)
        .await?;
        rows.into_iter()
            .map(|row| user(row.id, row.name, &row.role, row.expires_ms))
            .collect()
    }

    pub async fn user(&self, id: UserId) -> Result<Option<User>> {
        let row = sqlx::query!(
            r#"SELECT id AS "id: Uuid", name, role, expires_ms FROM users WHERE id = ?"#,
            id.0
        )
        .fetch_optional(&self.reader)
        .await?;
        row.map(|row| user(row.id, row.name, &row.role, row.expires_ms))
            .transpose()
    }

    /// Add a user; [`Error::Duplicate`] when the name is taken.
    pub async fn create_user(&self, user: NewUser<'_>) -> Result<()> {
        let (created, expires) = (millis(user.created), user.expires.map(millis));
        let role = user.role.code();
        let inserted = sqlx::query!(
            "INSERT INTO users (id, name, password_hash, role, expires_ms, created_ms)
             VALUES (?, ?, ?, ?, ?, ?)",
            user.id.0,
            user.name,
            user.password_hash,
            role,
            expires,
            created,
        )
        .execute(&self.writer)
        .await;
        match inserted {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(Error::Duplicate("user"))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Set a user's role and expiry, unless that demotes the last owner.
    pub async fn update_user(
        &self,
        id: UserId,
        role: Role,
        expires: Option<Timestamp>,
    ) -> Result<Change> {
        let (code, expires) = (role.code(), expires.map(millis));

        let mut tx = self.writer.begin().await?;
        if let Some(refused) = guard_owner(&mut tx, id, Some(role)).await? {
            return Ok(refused);
        }
        sqlx::query!(
            "UPDATE users SET role = ?, expires_ms = ? WHERE id = ?",
            code,
            expires,
            id.0,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Change::Done)
    }

    /// Delete a user, and with them their sessions and tokens, unless
    /// they are the last owner.
    pub async fn delete_user(&self, id: UserId) -> Result<Change> {
        let mut tx = self.writer.begin().await?;
        if let Some(refused) = guard_owner(&mut tx, id, None).await? {
            return Ok(refused);
        }
        sqlx::query!("DELETE FROM users WHERE id = ?", id.0)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Change::Done)
    }

    /// Replace the password and end other sessions in one transaction.
    /// A self-service change supplies the hash it verified, so a concurrent
    /// reset cannot be overwritten by a request using the old password.
    pub async fn set_password(
        &self,
        id: UserId,
        password_hash: &str,
        expected: Option<&str>,
        except: Option<&[u8; 32]>,
    ) -> Result<bool> {
        let mut tx = self.writer.begin().await?;
        let done = sqlx::query!(
            "UPDATE users SET password_hash = ? WHERE id = ?
             AND (? IS NULL OR password_hash = ?)",
            password_hash,
            id.0,
            expected,
            expected,
        )
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() != 1 {
            return Ok(false);
        }
        let keep = except.map(<[u8; 32]>::as_slice);
        sqlx::query!(
            "DELETE FROM sessions WHERE user_id = ? AND token_hash IS NOT ?",
            id.0,
            keep,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}

/// Whether the change of user `id` to `role` (`None`: deleting them) must
/// be refused, inside the change's transaction: `NotFound` for no such
/// user, `LastOwner` when they are the only owner and would stop being one.
async fn guard_owner(
    tx: &mut sqlx::SqliteConnection,
    id: UserId,
    role: Option<Role>,
) -> Result<Option<Change>> {
    let current = sqlx::query!("SELECT role FROM users WHERE id = ?", id.0)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(current) = current else {
        return Ok(Some(Change::NotFound));
    };
    if current.role != Role::Owner.code() || role == Some(Role::Owner) {
        return Ok(None);
    }

    let owner = Role::Owner.code();
    let owners = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!: i64" FROM users WHERE role = ?"#,
        owner
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok((owners.count <= 1).then_some(Change::LastOwner))
}

pub(crate) fn user(id: Uuid, name: String, role: &str, expires_ms: Option<i64>) -> Result<User> {
    Ok(User {
        id: UserId(id),
        name,
        role: Role::from_code(role).ok_or(Error::Corrupt("role"))?,
        expires: expires_ms.map(timestamp).transpose()?,
    })
}
