//! The token file: `login` makes a token from a password and keeps it,
//! `logout` revokes it and deletes the file, and every other verb sends
//! it.
//!
//! The file sits beside the device API's key file in the owner's data
//! directory ([`crate::home`]), mode
//! 0600 like every private file, and holds the server's address, the
//! account's name and the token, versioned as a file an owner keeps
//! ([`wellfed_formats::kept`]): one a newer build wrote is refused and never
//! written over or deleted. Only a login session can make a token, so
//! `login` opens one, makes the token and ends the session: nothing but
//! the token outlives the command, and revoking it is all a lost laptop
//! needs.

use std::fmt;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use serde::{Deserialize, Serialize};
use wellfed_formats::kept::{self, Found, Kept};
use wellfed_formats::private;
use wellfed_server_client::api::{Credentials, ErrorCode, NewToken, TokenId};
use wellfed_server_client::{Auth, Client, Error, Url};

use super::{Output, Settled};

/// The name a new token gets unless `--token-name` gives one.
const TOKEN_NAME: &str = "wellfed-cli";

#[derive(Args)]
pub(crate) struct LoginArgs {
    /// The server's address, like `http://192.0.2.7:8080`.
    #[arg(long)]
    server: Url,
    /// The account's name; its password is read from standard input.
    #[arg(long)]
    name: String,
    /// What the server lists the token as (1-64 characters).
    #[arg(long, default_value = TOKEN_NAME)]
    token_name: String,
}

/// The token file's major.
const VERSION: u32 = 1;

/// What the token file holds. No field is denied: a newer minor's
/// additions are skipped.
#[derive(Serialize, Deserialize)]
struct TokenFile {
    version: u32,
    /// The server's base URL, as `login` was given it.
    server: String,
    /// The account the token belongs to.
    user: String,
    /// The token's id, which `logout` revokes.
    id: TokenId,
    /// The bearer secret.
    secret: String,
}

impl fmt::Debug for TokenFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenFile")
            .field("server", &self.server)
            .field("user", &self.user)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Kept for TokenFile {
    const MAJOR: u32 = VERSION;
}

impl TokenFile {
    fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!(
                "not logged in: {} does not exist; run `wellfed server login` first",
                path.display()
            );
        }
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Self::read(&bytes).with_context(|| {
            format!(
                "read the token file {}; `wellfed server login` sets one that doesn't read aside",
                path.display()
            )
        })
    }

    fn read(bytes: &[u8]) -> Result<Self> {
        match kept::read::<Self>(bytes)? {
            Found::Read(file) => Ok(file),
            Found::Newer { major } => bail!(
                "it is version {major}, a newer build's (this one reads up to {VERSION}), and is left as it is"
            ),
        }
    }

    fn client(&self) -> Result<Client> {
        let base = Url::parse(&self.server)
            .with_context(|| format!("the token file's server {:?}", self.server))?;
        let client = Client::new(&base)?;
        Ok(client.with_auth(Some(Auth::Token(self.secret.clone()))))
    }
}

/// A client of the server the token file names, sending its token.
pub(super) fn client(path: &Path) -> Result<Client> {
    TokenFile::load(path)?.client()
}

/// Make way for a login's token file at `path`: refused while a token file
/// that reads is there (log out first) or one a newer build wrote (left as
/// it is). One that doesn't read at all (damaged, or from before the file
/// had a version) can't be logged out of, so it is set aside, never deleted,
/// and the path where it went returned.
fn make_way(path: &Path) -> Result<Option<PathBuf>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    match kept::read::<TokenFile>(&bytes) {
        Ok(Found::Read(kept)) => bail!(
            "already logged in to {} as {} ({}); run `wellfed server logout` first",
            kept.server,
            kept.user,
            path.display()
        ),
        Ok(Found::Newer { major }) => bail!(
            "{} is version {major}, a newer build's (this one reads up to {VERSION}), and is left as it is",
            path.display()
        ),
        Err(why) => {
            let aside = private::set_aside(path)
                .with_context(|| format!("set aside {}, which does not read", path.display()))?;
            eprintln!(
                "The token file {} does not read ({why}); kept as {}. If its token is still \
                 listed on the server, revoke it there.",
                path.display(),
                aside.display()
            );
            Ok(Some(aside))
        }
    }
}

/// Log in, make a token for this computer, keep it, and end the session.
pub(super) async fn login(args: LoginArgs, path: &Path, out: Output) -> Result<Settled> {
    make_way(path)?;
    let password = password()?;

    let mut client = Client::new(&args.server)?;
    let credentials = Credentials {
        name: args.name,
        password,
    };
    let user = client.login(&credentials).await?;
    let new = NewToken {
        name: args.token_name,
        expires: None,
    };
    let created = client.create_token(&new).await?;

    let file = TokenFile {
        version: VERSION,
        server: args.server.as_str().to_owned(),
        user: user.name.clone(),
        id: created.token.id,
        secret: created.secret,
    };
    let written = kept::write::<TokenFile>(path, &file)
        .with_context(|| format!("write the token file {}", path.display()));

    // A token no file holds could never be revoked from here, so it goes
    // through the session that made it.
    if written.is_err()
        && let Err(error) = client.revoke_token(file.id).await
    {
        eprintln!(
            "The new token {} was not revoked ({error}); revoke it with `wellfed server tokens revoke`.",
            file.id
        );
    }

    // The session that made the token is not needed again.
    if let Err(error) = client.logout().await {
        eprintln!("The login session did not end ({error}); it expires on its own.");
    }
    written?;

    // The token's secret is the file's alone: the body printed is the
    // token as the server lists it.
    out.body(&created.token)?;
    out.text(format!(
        "Logged in to {} as {}; token {:?} kept in {}.",
        file.server,
        user.name,
        created.token.name,
        path.display()
    ));
    Ok(Settled::Known)
}

/// Revoke the kept token and delete the file. A token the server no
/// longer knows (revoked, or expired) is as good as revoked.
pub(super) async fn logout(path: &Path, out: Output) -> Result<Settled> {
    let file = TokenFile::load(path)?;
    let client = file.client()?;

    match client.revoke_token(file.id).await {
        Ok(()) => out.text(format!("Token revoked; logged out of {}.", file.server)),
        Err(error) if gone(&error) => out.text(format!(
            "The token was already revoked or expired; logged out of {}.",
            file.server
        )),
        Err(error) => {
            return Err(error).context("the token was not revoked, so its file is kept");
        }
    }

    std::fs::remove_file(path).with_context(|| format!("delete {}", path.display()))?;
    Ok(Settled::Known)
}

/// Whether the server refused the token as no token at all.
fn gone(error: &Error) -> bool {
    matches!(
        error.code(),
        Some(ErrorCode::Unauthenticated | ErrorCode::NotFound)
    )
}

/// The account's password, the first line of standard input.
fn password() -> Result<String> {
    eprintln!("Password (standard input):");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("read the password")?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../tests/fixtures/token-v1.json");

    #[test]
    fn version_1_reads_and_writes_as_its_sample() {
        let file = TokenFile::read(SAMPLE.as_bytes()).unwrap();

        assert_eq!(file.server, "http://192.0.2.7:8080/");
        assert_eq!(file.user, "ada");
        assert_eq!(file.secret, "a-sample-token-secret");
        // No field can be absent, so the sample written back is every key
        // written (`wellfed_formats::kept`).
        let written = serde_json::to_value(&file).unwrap();
        let sample: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(written, sample);
    }

    #[test]
    fn a_newer_major_is_refused() {
        let Err(error) = TokenFile::read(br#"{"version":2}"#) else {
            panic!("a newer build's token file is refused");
        };
        assert!(error.to_string().contains("version 2"), "{error}");
    }

    /// A token file that doesn't read (here from before it had a version)
    /// is set aside whole, so logging in again is all it takes.
    #[test]
    fn a_login_sets_aside_a_token_file_that_does_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wellfed-server.json");
        let unversioned = br#"{"server": "http://192.0.2.7:8080/", "user": "ada"}"#;
        std::fs::write(&path, unversioned).unwrap();

        let aside = make_way(&path).unwrap().unwrap();

        assert!(!path.exists());
        assert_eq!(aside, dir.path().join("wellfed-server.json.damaged-1"));
        assert_eq!(std::fs::read(&aside).unwrap(), unversioned);
        assert_eq!(make_way(&path).unwrap(), None, "nothing in the way now");
    }

    #[test]
    fn a_login_is_refused_over_a_token_file_that_reads_or_is_newer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wellfed-server.json");

        for (held, says) in [
            (SAMPLE.as_bytes(), "already logged in"),
            (&br#"{"version": 2}"#[..], "newer build"),
        ] {
            std::fs::write(&path, held).unwrap();
            let Err(error) = make_way(&path) else {
                panic!("refused");
            };
            assert!(error.to_string().contains(says), "{error}");
            assert_eq!(std::fs::read(&path).unwrap(), held, "left as it was");
        }
    }
}
