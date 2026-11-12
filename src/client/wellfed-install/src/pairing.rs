//! Becoming a feeder's owner: Bluetooth setup over the platform's
//! [`SetupLink`], or the recovery window on the LAN. Both run
//! `wellfed-client`'s setup session and announce the one-time code the
//! owner keys in on the feeder's buttons; the feeder's proof of the code,
//! not the person's answer, decides the pairing.

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use wellfed_client::setup::{Ask, Credentials, Session, Step};
use wellfed_model::setup::{Enrolled, Refusal, Reply, WifiSaved};
use wellfed_session::setup_code::Code;

use crate::Error;
use crate::platform::{Announcement, Keys, Prompter, SetupLink};
use crate::session::random;

/// The feeder answers each handshake message at once.
const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);
/// Beyond the feeder's own confirmation window, for the reply to arrive.
const CONFIRM_SLACK: Duration = Duration::from_secs(10);

/// What the feeder said when setup ended well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub enrolled: Enrolled,
    pub wifi: WifiSaved,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let enrolled = match self.enrolled {
            Enrolled::New => "enrolled as an owner",
            Enrolled::Already => "already enrolled",
        };
        let wifi = match self.wifi {
            WifiSaved::Saved => "Wi-Fi saved for its next join",
            WifiSaved::Unchanged => "Wi-Fi unchanged",
        };
        write!(f, "{enrolled}, {wifi}")
    }
}

/// Why the feeder refused a setup, as the owner reads it.
pub(crate) fn refusal_text(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::Busy => "another setup is waiting for its code",
        Refusal::Unconfirmed => "the code was not keyed in in time",
        Refusal::WrongCode => {
            "the code keyed in does not match: mistyped, or another device is relaying"
        }
        Refusal::Full => "every client slot is taken; revoke one first",
        Refusal::NotWritable => "its store cannot be written now",
        Refusal::Invalid => "it rejected the label or the Wi-Fi credentials",
    }
}

/// A setup session for `keys`, giving the feeder `wifi` when set, with a
/// fresh code.
fn session(keys: &impl Keys, wifi: Option<Credentials>) -> Result<Session, Error> {
    let ask = Ask {
        label: keys.label().to_string(),
        wifi,
    };
    let ephemeral: [u8; 32] = random()?;
    let seed: [u8; 32] = random()?;
    let bits: [u8; 2] = random()?;
    let code = Code::from_bits(u16::from_le_bytes(bits));
    Ok(Session::new(keys.keypair(), ephemeral, ask, code, seed)?)
}

/// What a reply means for the exchange: the code to key in, the end, or a
/// refusal.
enum Next {
    EnterCode(u16),
    Done(Outcome),
}

fn next(reply: Reply) -> Result<Next, Error> {
    match reply {
        Reply::EnterCode { seconds } => Ok(Next::EnterCode(seconds)),
        Reply::Done { enrolled, wifi } => Ok(Next::Done(Outcome { enrolled, wifi })),
        Reply::Refused(refusal) => Err(Error::SetupRefused(refusal)),
    }
}

/// Enroll `keys` as an owner over `link` to a feeder in setup mode, giving
/// it `wifi` when set, the owner keying in the code `prompter` announces;
/// the feeder's identity, remembered by `keys`, and what the feeder said.
/// The link is closed either way.
pub async fn over_link(
    keys: &mut impl Keys,
    wifi: Option<Credentials>,
    mut link: impl SetupLink,
    prompter: &mut impl Prompter,
) -> Result<([u8; 32], Outcome), Error> {
    let mut session = session(keys, wifi)?;
    let result = exchange(&mut session, &mut link, prompter).await;
    link.close().await;
    let outcome = result.map_err(|error| unsettled(keys, &session, error))?;

    let identity = session.feeder_key().ok_or(Error::NoProof)?;
    keys.remember(&identity).map_err(Error::Platform)?;
    Ok((identity, outcome))
}

/// What an exchange that ended in `error` leaves. Once the feeder has proven
/// the code and the last step went out, the feeder goes on without the
/// client: a link or a wait that ends then says nothing of the outcome. The
/// proof makes the identity trustworthy, so it is remembered, and the error
/// says the setup is unsettled rather than failed. A refusal is the
/// feeder's own answer, and stays one.
fn unsettled(keys: &mut impl Keys, session: &Session, error: Error) -> Error {
    let Some(identity) = session.feeder_key() else {
        return error;
    };
    if matches!(error, Error::SetupRefused(_)) {
        return error;
    }
    if let Err(platform) = keys.remember(&identity) {
        return Error::Platform(platform);
    }
    Error::Unsettled {
        identity,
        source: Box::new(error),
    }
}

/// Run the session over `link` to its end.
async fn exchange(
    session: &mut Session,
    link: &mut impl SetupLink,
    prompter: &mut impl Prompter,
) -> Result<Outcome, Error> {
    link.send(&session.start()?)
        .await
        .map_err(Error::Platform)?;
    let mut wait = HANDSHAKE_WAIT;
    loop {
        let message = link.receive(wait).await.map_err(Error::Platform)?;
        match session.receive(&message)? {
            Step::Send(bytes) => link.send(&bytes).await.map_err(Error::Platform)?,
            Step::Reply(reply) => match next(reply)? {
                Next::EnterCode(seconds) => {
                    show_code(prompter, session.code(), seconds).await?;
                    wait = Duration::from_secs(seconds.into()) + CONFIRM_SLACK;
                }
                Next::Done(outcome) => return Ok(outcome),
            },
        }
    }
}

/// Announce the code to key in. The feeder already waits for it when it
/// asks, so the announcement goes at once and the run waits only for the
/// feeder's outcome, within its window.
async fn show_code(prompter: &mut impl Prompter, code: Code, seconds: u16) -> Result<(), Error> {
    prompter
        .announce(Announcement::SetupCode { code, seconds })
        .await
        .map_err(Error::ShowCode)
}

/// Enroll `keys` as an owner through the feeder's recovery window at
/// `address`, the owner keying in the code `prompter` announces; the
/// feeder's identity, remembered by `keys`, and what the feeder said.
pub async fn recover(
    keys: &mut impl Keys,
    address: SocketAddr,
    prompter: &mut impl Prompter,
) -> Result<([u8; 32], Outcome), Error> {
    let session = session(keys, None)?;
    let mut recovery = wellfed_connect::Recovery::open(address, session)
        .await
        .map_err(|source| Error::Connect { address, source })?;

    let mut wait = HANDSHAKE_WAIT;
    let outcome = loop {
        let reply = match recovery.reply(wait).await {
            Ok(reply) => reply,
            Err(error) => {
                let error = match error {
                    wellfed_connect::Error::Closed => Error::RecoveryClosed,
                    wellfed_connect::Error::Timeout => Error::RecoveryTimeout,
                    other => other.into(),
                };
                return Err(unsettled(keys, recovery.session(), error));
            }
        };
        match next(reply)? {
            Next::EnterCode(seconds) => {
                show_code(prompter, recovery.session().code(), seconds).await?;
                wait = Duration::from_secs(seconds.into()) + CONFIRM_SLACK;
            }
            Next::Done(outcome) => break outcome,
        }
    };

    let identity = recovery.session().feeder_key().ok_or(Error::NoProof)?;
    keys.remember(&identity).map_err(Error::Platform)?;
    Ok((identity, outcome))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use wellfed_device_api::setup::{MAX_MESSAGE, Received, Responder};
    use wellfed_session::noise::Keypair;

    use super::*;
    use crate::platform::PlatformError;
    use crate::prompt::Prompt;

    /// This client's key, and the feeders it was told enrolled it.
    struct Remembering {
        keypair: Keypair,
        remembered: Vec<[u8; 32]>,
    }

    impl Keys for Remembering {
        fn keypair(&self) -> &Keypair {
            &self.keypair
        }

        fn label(&self) -> &str {
            "phone"
        }

        fn feeder(&self, _prefix: Option<&str>) -> Option<[u8; 32]> {
            None
        }

        fn remember(&mut self, identity: &[u8; 32]) -> Result<(), PlatformError> {
            self.remembered.push(*identity);
            Ok(())
        }
    }

    /// The owner at the panel: the code the client announces is the one
    /// keyed in.
    struct Owner(Rc<Cell<Option<Code>>>);

    impl Prompter for Owner {
        async fn done(&mut self, _prompt: Prompt) -> Result<(), PlatformError> {
            Ok(())
        }

        async fn yes(&mut self, _prompt: Prompt) -> Result<bool, PlatformError> {
            Ok(false)
        }

        async fn announce(&mut self, announcement: Announcement) -> Result<(), PlatformError> {
            if let Announcement::SetupCode { code, .. } = announcement {
                self.0.set(Some(code));
            }
            Ok(())
        }

        async fn settle(&mut self) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    /// A feeder in setup mode behind a link that drops once the client's
    /// last step arrives: the feeder goes on without it.
    struct Dropping {
        feeder: Responder,
        keyed: Rc<Cell<Option<Code>>>,
        /// The feeder's next message to the client.
        next: Option<Vec<u8>>,
        proven: bool,
    }

    impl SetupLink for Dropping {
        async fn send(&mut self, message: &[u8]) -> Result<(), PlatformError> {
            let mut out = [0u8; MAX_MESSAGE];
            match self
                .feeder
                .receive(message, &mut out)
                .map_err(feeder_error)?
            {
                Received::Send(len) => self.next = Some(out[..len].to_vec()),
                Received::Offer(_) => {
                    let reply = wellfed_proto::setup::Reply::EnterCode { seconds: 60 };
                    let len = self.feeder.reply(&reply, &mut out).map_err(feeder_error)?;
                    self.next = Some(out[..len].to_vec());
                }
                Received::Proven { .. } => self.proven = true,
            }
            Ok(())
        }

        async fn receive(&mut self, _wait: Duration) -> Result<Vec<u8>, PlatformError> {
            if self.proven {
                return Err("the link dropped".into());
            }
            if let Some(next) = self.next.take() {
                return Ok(next);
            }
            // The code is in: the feeder starts the proof.
            let code = self.keyed.take().ok_or("no code was keyed in")?;
            let mut out = [0u8; MAX_MESSAGE];
            let len = self
                .feeder
                .prove(code, [6; 32], &mut out)
                .map_err(feeder_error)?;
            Ok(out[..len].to_vec())
        }

        async fn close(self) {}
    }

    fn feeder_error(error: wellfed_device_api::setup::Error) -> PlatformError {
        format!("the feeder: {error:?}").into()
    }

    #[tokio::test]
    async fn a_link_lost_after_the_last_step_is_unsettled_and_keeps_the_proven_feeder() {
        let feeder = Keypair::from_secret([1; 32]);
        let mut keys = Remembering {
            keypair: Keypair::from_secret([2; 32]),
            remembered: Vec::new(),
        };
        let keyed = Rc::new(Cell::new(None));
        let link = Dropping {
            feeder: Responder::new(&feeder, [4; 32]).unwrap(),
            keyed: Rc::clone(&keyed),
            next: None,
            proven: false,
        };
        let wifi = Credentials::new(b"Lab".to_vec(), "pass word!".into()).unwrap();

        let paired = over_link(&mut keys, Some(wifi), link, &mut Owner(keyed)).await;

        let Err(Error::Unsettled { identity, source }) = paired else {
            panic!("a lost link after the last step is not a failure: {paired:?}");
        };
        assert_eq!(identity, *feeder.public());
        assert!(matches!(*source, Error::Platform(_)), "{source:?}");
        assert_eq!(keys.remembered, [*feeder.public()]);
    }
}
