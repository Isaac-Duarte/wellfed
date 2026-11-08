//! Who may use the feeder: invites, joining with one, and the enrolled
//! clients, which an owner lists and revokes.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use wellfed_client::api::Invite;
use wellfed_model::{Answer, ClientRole, Joined, Op};

use crate::keys::fingerprint;
use crate::{Target, connect, device, print_answer};

#[derive(Clone, Copy, ValueEnum)]
enum InviteRole {
    Owner,
    Member,
    Server,
}

#[derive(Args)]
pub(crate) struct InviteArgs {
    #[command(flatten)]
    target: Target,
    #[arg(long, value_enum, default_value = "member")]
    role: InviteRole,
}

#[derive(Args)]
pub(crate) struct JoinArgs {
    /// The invite as `wellfed invite` printed it.
    invite: String,
    /// The key that joins [default: wellfed-owner.json in
    /// WELLFED_HOME, or the platform's data directory].
    #[arg(long)]
    keys: Option<PathBuf>,
    /// Make a new key at that path if there is none. Without it, a missing
    /// key is refused, so a command pointed at the wrong place never makes a
    /// second key.
    #[arg(long)]
    create_key: bool,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    address: Option<String>,
}

#[derive(Args)]
pub(crate) struct RevokeArgs {
    #[command(flatten)]
    target: Target,
    /// The client's key, as a hex prefix of at least 8 digits.
    client: String,
}

pub(crate) async fn invite_command(args: InviteArgs) -> Result<()> {
    let role = match args.role {
        InviteRole::Owner => ClientRole::Owner,
        InviteRole::Member => ClientRole::Member,
        InviteRole::Server => ClientRole::Server,
    };
    let (_, identity, mut connection) = connect(&args.target).await?;
    match connection.ask(&Op::CreateInvite { role }).await? {
        Answer::Invite(invite) => {
            let text = Invite {
                feeder: identity,
                id: invite.id,
                secret: invite.secret.0,
            }
            .text();
            println!("{text}");
            eprintln!(
                "Single use, for the next {} s; anyone holding it joins as {role:?}.",
                invite.seconds
            );
            Ok(())
        }
        Answer::Refused(refusal) => bail!("refused: {refusal:?}"),
        other => bail!("unexpected answer {other:?}"),
    }
}

pub(crate) async fn join_command(args: JoinArgs) -> Result<()> {
    let invite = Invite::parse(&args.invite)?;
    let mut key =
        crate::pairing::open_key(args.keys.as_deref(), args.label.as_deref(), args.create_key)?;
    let address = device::locate(&invite.feeder, args.address.as_deref())?;
    let joined = wellfed_connect::join(
        key.keypair(),
        invite,
        key.label(),
        address,
        device::random()?,
    )
    .await
    .map_err(|error| match error {
        wellfed_connect::Error::Closed => {
            anyhow::anyhow!("the feeder closed the connection (an unknown or expired invite)")
        }
        other => other.into(),
    })?;
    match joined {
        Joined::Enrolled { role, .. } => {
            key.remember(&invite.feeder)?;
            println!("Joined feeder {} as {role:?}.", fingerprint(&invite.feeder));
            Ok(())
        }
        Joined::Refused(refusal) => bail!("refused: {refusal:?}"),
    }
}

pub(crate) async fn clients_command(target: Target) -> Result<()> {
    let (_, _, mut connection) = connect(&target).await?;
    let Answer::Clients(clients) = connection.ask(&Op::ListClients).await? else {
        bail!("the feeder did not list its clients");
    };
    for client in &clients {
        println!(
            "{}  {:<6?}  {}{}",
            client.key,
            client.role,
            client.label,
            if client.you { "  (this key)" } else { "" }
        );
    }
    Ok(())
}

pub(crate) async fn revoke_command(args: RevokeArgs) -> Result<()> {
    if args.client.len() < 8 {
        bail!("give at least 8 hex digits of the client's key");
    }
    let (_, _, mut connection) = connect(&args.target).await?;
    let Answer::Clients(clients) = connection.ask(&Op::ListClients).await? else {
        bail!("the feeder did not list its clients");
    };
    let matches: Vec<[u8; 32]> = clients
        .iter()
        .filter(|c| c.key.to_string().starts_with(&args.client))
        .map(|c| c.key.0)
        .collect();
    let [key] = matches.as_slice() else {
        bail!("{} clients match {}", matches.len(), args.client);
    };
    let answer = connection.ask(&Op::RevokeClient { key: *key }).await?;
    print_answer(&answer)
}
