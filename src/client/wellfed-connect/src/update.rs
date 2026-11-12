//! Pushing a signed update until bank 1 is activated: the tokio driver of
//! [`wellfed_client::update::Installer`], shared by the owner's CLI (its
//! installer's promotion and "Restore stock") and the server's installs.
//!
//! The steps are the installer's; the driver only moves them over whatever
//! carries requests to the feeder ([`Steps`]): the CLI's own session, which
//! it reopens itself, or the server's feeder task, which keeps reconnecting
//! on its own while the install waits on its long-operation lane. A dropped
//! session loses nothing: chunks are idempotent by offset and bank 0 waits a
//! minute for the next, so a lost answer or a restart reopens the session
//! and the install resumes where bank 0 says. An offer is never sent again
//! behind the caller's back: a refusal ends the push.

use std::time::Duration;

use wellfed_client::update::{Image, InstallError, Installer, Next, Sent};
use wellfed_model::{Answer, Op, UpdateView};

/// Sessions a push reopens at most: a feeder that keeps dropping them is
/// not coming back.
pub const REOPEN_MAX: u32 = 20;

/// How long the driver waits between its steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    /// While bank 0 writes a sector and wants nothing: before reading its
    /// state again.
    pub settle: Duration,
    /// After bank 1 gives way: before looking for the session bank 0
    /// answers, so the restart has begun.
    pub restart: Duration,
}

impl Pace {
    pub const DEFAULT: Self = Self {
        settle: Duration::from_millis(200),
        restart: Duration::from_secs(5),
    };
}

/// A session that opened: the bank its update state names, and the build
/// its welcome names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub bank: u8,
    pub build: String,
}

/// What carries a push's requests to the feeder.
#[expect(
    async_fn_in_trait,
    reason = "each caller drives its own concrete type, so the futures' Send is known where it \
              matters (the server's spawned install)"
)]
pub trait Steps {
    /// A failure of the caller's own (its random source, its store), which
    /// ends the push; the feeder's answers and a dropped session never are.
    type Error;

    /// Send `op` under a fresh request id and wait for its answer; `None`
    /// when the session ended or the answer did not come in time, so the
    /// request may have run.
    async fn step(&mut self, op: Op) -> Result<Option<Answer>, Self::Error>;

    /// A session that answers an update-state read (the bank) and its
    /// welcome (the build), waiting as long as a restarting feeder takes;
    /// `None` once the feeder did not come back. Never the status: bank 0
    /// may run a build of another schema revision, which only the frozen
    /// update path reaches.
    async fn reopen(&mut self) -> Result<Option<Opened>, Self::Error>;

    /// Whether a session that `opened` shows bank 1 running the build:
    /// before the offer, it already ran it; after, the activation's answer
    /// was lost and the feeder restarted into its trial boot, where
    /// offering the manifest again would interrupt the trial.
    fn activated(&mut self, opened: &Opened) -> bool;

    /// The feeder's update state, after each answer that carried one.
    fn progress(&mut self, view: &UpdateView);
}

/// How a push ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// Bank 1 is activated; the feeder restarts into it.
    Activated,
    /// Bank 1 already ran the build before anything was offered: nothing
    /// was sent, and nothing needs to be.
    AlreadyRunning,
    /// The install's own error, for good.
    Failed(InstallError),
    /// The feeder stopped answering the device API, or answered in a way
    /// this build cannot read. With `offered`, the offer went out: bank 1
    /// may be partly written, or activated with its answer lost, and only
    /// the feeder can tell.
    Gone { offered: bool },
}

/// Push `image` until bank 1 is activated (or already runs it), the
/// install fails, or the feeder is gone.
pub async fn push<S: Steps>(
    image: Image<'_>,
    steps: &mut S,
    pace: Pace,
) -> Result<Pushed, S::Error> {
    let mut installer = Installer::new(image);
    let mut open = false;
    let mut reopened = 0;
    let mut offered = false;
    loop {
        if !open {
            if reopened >= REOPEN_MAX {
                return Ok(Pushed::Gone { offered });
            }
            reopened += 1;
            let Some(opened) = steps.reopen().await? else {
                return Ok(Pushed::Gone { offered });
            };
            // Before the offer, bank 1 ran the build all along; after it,
            // the activation's answer was lost.
            if steps.activated(&opened) {
                let pushed = if offered {
                    Pushed::Activated
                } else {
                    Pushed::AlreadyRunning
                };
                return Ok(pushed);
            }
            installer.opened(opened.bank);
            open = true;
        }

        match installer.poll() {
            Next::Send(sent, op) => {
                offered |= sent == Sent::Offer;
                match steps.step(op).await? {
                    // Only a state or a refusal says where the install
                    // stands. An answer this build cannot read (a newer
                    // build's), or of another kind, may follow an offer
                    // that took: the push ends unsettled, as a feeder that
                    // went silent does, and nothing is sent again.
                    Some(answer) if !matches!(answer, Answer::Update(_) | Answer::Refused(_)) => {
                        return Ok(Pushed::Gone { offered });
                    }
                    // An answer the installer rejects ends the install; the
                    // next poll says so.
                    Some(answer) => {
                        let _ = installer.answer(sent, &answer);
                        if let (Answer::Update(_), Some(view)) = (&answer, installer.view()) {
                            steps.progress(view);
                        }
                    }
                    None => {
                        installer.lost();
                        open = false;
                    }
                }
            }
            Next::Wait => tokio::time::sleep(pace.settle).await,
            Next::Activated => return Ok(Pushed::Activated),
            Next::Failed(error) => return Ok(Pushed::Failed(error)),
            // Bank 1 gave way (a feeder that answered from bank 1):
            // reconnect once it has restarted into bank 0.
            Next::Reconnect => {
                installer.lost();
                open = false;
                tokio::time::sleep(pace.restart).await;
            }
            // Nothing here cancels.
            Next::Cancelled => return Ok(Pushed::Failed(InstallError::Lost)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    /// A feeder that answers every request with `answer`, and keeps what it
    /// was sent.
    struct Answering {
        answer: Answer,
        sent: Vec<Op>,
    }

    impl Steps for Answering {
        type Error = Infallible;

        async fn step(&mut self, op: Op) -> Result<Option<Answer>, Infallible> {
            self.sent.push(op);
            Ok(Some(self.answer.clone()))
        }

        async fn reopen(&mut self) -> Result<Option<Opened>, Infallible> {
            Ok(Some(Opened {
                bank: 0,
                build: "bank 0".into(),
            }))
        }

        fn activated(&mut self, _: &Opened) -> bool {
            false
        }

        fn progress(&mut self, _: &UpdateView) {}
    }

    /// An offer answered in a way this build cannot read, or with an answer
    /// of another kind, may have taken: the push ends unsettled after the
    /// one offer, never as a refusal, and offers nothing again.
    #[test]
    fn an_unreadable_answer_to_the_offer_leaves_the_push_unsettled() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let image = Image {
            manifest: b"manifest",
            downgrade: None,
            payload: &[1; 64],
        };

        for answer in [Answer::Unreadable, Answer::Done { generation: None }] {
            let mut feeder = Answering {
                answer: answer.clone(),
                sent: Vec::new(),
            };
            let pushed = runtime.block_on(push(image, &mut feeder, Pace::DEFAULT));

            assert_eq!(pushed, Ok(Pushed::Gone { offered: true }), "{answer:?}");
            assert!(
                matches!(feeder.sent[..], [Op::OfferUpdate { .. }]),
                "{answer:?}: {:?}",
                feeder.sent
            );
        }
    }
}
