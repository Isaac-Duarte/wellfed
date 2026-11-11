//! The owner's update-signing key: create one, enroll it on a feeder with
//! a Dispense press there, and list the keys a feeder trusts this way.
//!
//! A feeder admits an update signed by a key built into its image or by
//! one an owner enrolled, so an enrolled key lets the owner sign their own
//! builds (`wellfed-build sign --key`) for that feeder. The bench key is
//! the same file, so it enrolls too: a feeder whose image embeds no key
//! then still takes the bench's signed updates. The enrollment itself is
//! `wellfed_install::update_key`'s, the one the app's install runs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use wellfed_client::update_key::{self, EnrollError, Enrolled};
use wellfed_formats::credentials::KeyScope;
use wellfed_model::{Client, ClientRole, Op};
use wellfed_session::hex;
use wellfed_signing::Key;

use crate::ask::Asker;
use crate::device::Connection;
use crate::report::Printer;
use crate::{Target, connect};

#[derive(Args)]
pub(crate) struct UpdateKeyArgs {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Create an owner's update-signing key; never replaces a file.
    New {
        /// Where to write it. It holds the private key: keep it private.
        #[arg(long)]
        output: PathBuf,
    },
    /// Enroll a key on the feeder: someone presses Dispense there within
    /// 60 s to confirm it. Owners only.
    Enroll(EnrollArgs),
    /// The update keys the feeder has enrolled.
    Show(Target),
    /// Send key statements a built-in or the release key signed offline,
    /// in order over one session: keys the feeder then trusts or stops
    /// trusting, with no press there. A rotation's revoke and add go
    /// together, so nothing is offered between them. Every file is read
    /// before anything is sent, and the first refusal or failure stops the
    /// rest; nothing is sent again.
    Statement {
        #[command(flatten)]
        target: Target,
        /// The signed statements, sent in this order.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[derive(Args)]
pub(crate) struct EnrollArgs {
    #[command(flatten)]
    target: Target,
    /// The key file: one `update-key new` wrote, or the bench key.
    #[arg(long)]
    key: PathBuf,
    /// The name the feeder lists it under.
    #[arg(long, default_value = "update-key", value_parser = label)]
    label: String,
}

fn label(text: &str) -> Result<String, EnrollError> {
    update_key::label(text)?;
    Ok(text.to_owned())
}

pub(crate) async fn command(args: UpdateKeyArgs) -> Result<()> {
    match args.action {
        Action::New { output } => {
            let public = wellfed_signing::keygen(&output, KeyScope::Owner)
                .with_context(|| format!("write {}", output.display()))?;
            println!(
                "Wrote update key {}; public key {public}.",
                output.display()
            );
            eprintln!("Enroll it on each feeder with `wellfed update-key enroll --key`.");
            Ok(())
        }
        Action::Enroll(args) => {
            let (_, _, mut connection) = connect(&args.target).await?;
            enroll(&mut connection, &args.key, &args.label).await
        }
        Action::Show(target) => {
            let (_, _, mut connection) = connect(&target).await?;
            show(&mut connection).await
        }
        Action::Statement { target, files } => {
            let statements = read_statements(&files)?;
            let (_, _, mut connection) = connect(&target).await?;
            for (file, statement) in files.iter().zip(statements) {
                let answer = connection
                    .ask(&Op::UpdateKeyStatement { statement })
                    .await
                    .with_context(|| format!("send {}", file.display()))?;
                crate::print_answer(&answer).with_context(|| file.display().to_string())?;
            }
            Ok(())
        }
    }
}

/// Every statement, read before any is sent: a file that cannot be read
/// must not leave a rotation half done.
fn read_statements(files: &[PathBuf]) -> Result<Vec<Vec<u8>>> {
    files
        .iter()
        .map(|file| std::fs::read(file).with_context(|| format!("read {}", file.display())))
        .collect()
}

/// Enroll the key in `file` over `connection`, the request a press at the
/// feeder answers sent at most once.
async fn enroll(connection: &mut Connection, file: &Path, label: &str) -> Result<()> {
    let key = Key::load(file).with_context(|| format!("load {}", file.display()))?;
    // The release key already signs for every consumer feeder and stays on
    // offline media; enrolling it would bring it onto an online machine.
    if key.scope() == KeyScope::Release {
        bail!(
            "{} is the release key; enroll an owner key instead",
            file.display()
        );
    }
    let id = hex::Lower(&key.id()).to_string();
    let outcome = wellfed_install::update_key::enroll(
        connection,
        key.public(),
        key.id(),
        label,
        &mut Asker::terminal_notices(),
        &Printer,
    )
    .await?;
    match outcome {
        Ok(Enrolled::Now) => println!("Enrolled update key {id} as {label:?}."),
        Ok(Enrolled::Already) => {
            println!("The feeder already trusts update key {id}; nothing sent.")
        }
        // A pre-revision build cannot list its clients, so `show` has
        // nothing to check with until the feeder is updated.
        Err(EnrollError::Uncertain) if wellfed_install::session::Session::legacy(connection) => {
            bail!(
                "no answer came to the enrollment of update key {id}: a press may have \
                 enrolled it all the same. The feeder runs a build from before API revisions, \
                 so update it first, then check with `wellfed update-key show`; the request is \
                 not sent again"
            )
        }
        Err(EnrollError::Uncertain) => bail!(
            "no answer came to the enrollment of update key {id}: a press may have enrolled \
             it all the same. Check with `wellfed update-key show`; the request is not sent \
             again"
        ),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

async fn show(connection: &mut Connection) -> Result<()> {
    let signers: Vec<Client> = wellfed_install::update_key::clients(connection)
        .await?
        .into_iter()
        .filter(|client| client.role == ClientRole::UpdateSigner)
        .collect();
    if signers.is_empty() {
        eprintln!("The feeder has no enrolled update keys.");
    }
    for signer in &signers {
        let id = wellfed_client::release::key_id(&signer.key.0);
        println!(
            "{}  key id {}  {}",
            signer.key,
            hex::Lower(&id),
            signer.label
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that cannot be read stops the statements before any is sent.
    #[test]
    fn every_statement_is_read_before_any_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let revoke = dir.path().join("revoke.bin");
        std::fs::write(&revoke, b"revoke").unwrap();
        let add = dir.path().join("add.bin");

        let error = read_statements(&[revoke.clone(), add.clone()]).unwrap_err();
        assert!(format!("{error:#}").contains("add.bin"), "{error:#}");

        std::fs::write(&add, b"add").unwrap();
        assert_eq!(
            read_statements(&[revoke, add]).unwrap(),
            [b"revoke".to_vec(), b"add".to_vec()]
        );
    }
}
