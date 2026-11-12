//! Who may use the feeder: invites, joining with one, and the enrolled
//! clients, which an owner lists and revokes.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use wellfed_client::api::Invite;
use wellfed_connect::Connection;
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
    .await;
    let joined = match joined {
        Ok(joined) => joined,
        Err(wellfed_connect::Error::Closed) => {
            bail!("the feeder closed the connection (an unknown or expired invite)")
        }
        // The join went and its answer was lost: a session says whether
        // the key was enrolled.
        Err(wellfed_connect::Error::JoinUnanswered) => {
            match Connection::open(key.keypair(), invite.feeder, address, device::random()?).await {
                Ok(session) => Joined::Enrolled {
                    role: session.role(),
                    model: session.model(),
                },
                Err(wellfed_connect::Error::UnknownKey) => bail!(LOST_JOIN_UNKNOWN),
                Err(error) => {
                    key.remember(&invite.feeder)?;
                    bail!("{LOST_JOIN_UNCHECKED}: {error}")
                }
            }
        }
        Err(error) => return Err(error.into()),
    };
    match joined {
        Joined::Enrolled { role, .. } => {
            key.remember(&invite.feeder)?;
            println!("Joined feeder {} as {role:?}.", fingerprint(&invite.feeder));
            Ok(())
        }
        Joined::Refused(refusal) => bail!("refused: {refusal:?}"),
    }
}

/// A join whose answer was lost, and a session then refused the key. The
/// feeder checks an invite's secret only at the handshake's third message,
/// so a wrong secret ends the join just as a lost answer does.
const LOST_JOIN_UNKNOWN: &str = "the join's answer was lost, and the feeder does not know this key: \
    the invite was refused (a wrong invite secret looks the same) or the enrollment did not land; \
    run `wellfed join` again with a new invite";

/// A join whose answer was lost, and no session could check it.
const LOST_JOIN_UNCHECKED: &str = "the join's answer was lost and a session to check it failed, \
    so the key may be enrolled (`wellfed status` tells)";

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
    let prefix = client_prefix(&args.client)?;
    let (_, _, mut connection) = connect(&args.target).await?;
    let Answer::Clients(clients) = connection.ask(&Op::ListClients).await? else {
        bail!("the feeder did not list its clients");
    };
    let chosen = one_client(&clients, |c| c.key.to_string(), &prefix)?;
    let answer = connection
        .ask(&Op::RevokeClient { key: chosen.key.0 })
        .await?;
    print_answer(&answer)
}

/// A client key's prefix as typed, in either case: at least 8 hex digits,
/// lowercased to match the keys as they print.
pub(crate) fn client_prefix(typed: &str) -> Result<String> {
    if typed.len() < 8 || !typed.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("give at least 8 hex digits of the client's key");
    }
    Ok(typed.to_ascii_lowercase())
}

/// The one client whose key, as `key` prints it, starts with `prefix`.
pub(crate) fn one_client<'a, T>(
    clients: &'a [T],
    key: impl Fn(&T) -> String,
    prefix: &str,
) -> Result<&'a T> {
    let matching: Vec<&T> = clients
        .iter()
        .filter(|client| key(client).starts_with(prefix))
        .collect();
    let [chosen] = matching.as_slice() else {
        bail!("{} clients match {prefix}", matching.len());
    };
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both revokes take a client's key prefix in either case, and at
    /// least 8 hex digits of it.
    #[test]
    fn a_client_prefix_matches_in_either_case() {
        let keys = [
            "abcdef12aa".to_owned(),
            "abcdef12bb".to_owned(),
            "12345678cc".to_owned(),
        ];
        let prefix = client_prefix("ABCDEF12BB").unwrap();
        assert_eq!(
            one_client(&keys, Clone::clone, &prefix).unwrap(),
            "abcdef12bb"
        );
        let prefix = client_prefix("abcdef12").unwrap();
        assert!(
            one_client(&keys, Clone::clone, &prefix).is_err(),
            "two match"
        );
        assert!(client_prefix("abcdef1").is_err());
        assert!(client_prefix("abcdefgh").is_err());
    }

    /// A wrapped message reads as one sentence, with no run of spaces
    /// where its source line broke.
    #[test]
    fn a_lost_join_s_messages_read_as_one_sentence() {
        assert_eq!(
            LOST_JOIN_UNKNOWN,
            "the join's answer was lost, and the feeder does not know this key: the invite was \
             refused (a wrong invite secret looks the same) or the enrollment did not land; run \
             `wellfed join` again with a new invite"
        );
        for message in [LOST_JOIN_UNKNOWN, LOST_JOIN_UNCHECKED] {
            assert!(!message.contains("  "), "{message}");
        }
    }
}
