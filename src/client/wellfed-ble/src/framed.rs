//! Whole setup messages over a control characteristic: each one framed by
//! `wellfed_client::setup` and written in [`PIECE`]-byte pieces, and the
//! notifications reassembled into messages. The radio is behind
//! [`Control`], so a test drives the framing over a fake peripheral.

use std::future::Future;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::time::timeout;
use wellfed_client::setup::{Deframer, frame};

use crate::{Error, Notifications, PIECE};

/// Where pieces go: a connected feeder's control characteristic.
pub trait Control {
    /// Write one piece, acknowledged before this returns, so pieces arrive
    /// in order.
    fn write(&self, piece: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
}

/// A [`Control`] and its notifications, carrying whole messages.
pub struct Framed<C> {
    control: C,
    notifications: Notifications,
    deframer: Deframer,
}

impl<C: Control> Framed<C> {
    pub fn new(control: C, notifications: Notifications) -> Self {
        Self {
            control,
            notifications,
            deframer: Deframer::default(),
        }
    }

    /// Send one message, framed, piece by piece.
    pub async fn send(&mut self, message: &[u8]) -> Result<(), Error> {
        for piece in frame(message)?.chunks(PIECE) {
            self.control.write(piece).await?;
        }
        Ok(())
    }

    /// The next whole message, waiting at most `wait`. A message the
    /// notifications already finished is returned without waiting.
    pub async fn receive(&mut self, wait: Duration) -> Result<Vec<u8>, Error> {
        timeout(wait, async {
            loop {
                if let Some(message) = self.deframer.next_message()? {
                    return Ok(message);
                }
                let notification = self.notifications.next().await.ok_or(Error::Dropped)?;
                self.deframer.push(&notification);
            }
        })
        .await
        .map_err(|_| Error::Silent)?
    }

    /// The control, to hang up with.
    pub fn into_control(self) -> C {
        self.control
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures_util::stream;
    use tokio::sync::mpsc;

    use super::*;

    /// A peripheral that keeps what was written and fails a write on
    /// request.
    #[derive(Clone, Default)]
    struct Fake {
        written: Arc<Mutex<Vec<Vec<u8>>>>,
        broken: bool,
    }

    impl Control for Fake {
        async fn write(&self, piece: &[u8]) -> Result<(), Error> {
            if self.broken {
                return Err(Error::WriteTimeout);
            }
            self.written.lock().unwrap().push(piece.to_vec());
            Ok(())
        }
    }

    fn notifications(pieces: Vec<Vec<u8>>) -> Notifications {
        Box::pin(stream::iter(pieces))
    }

    #[tokio::test]
    async fn a_message_goes_framed_in_pieces_that_fit_the_smallest_mtu() {
        let fake = Fake::default();
        let mut framed = Framed::new(fake.clone(), notifications(vec![]));
        let message: Vec<u8> = (0..=60).collect();

        framed.send(&message).await.unwrap();

        let written = fake.written.lock().unwrap().clone();
        assert!(written.iter().all(|piece| piece.len() <= PIECE));
        assert_eq!(written.concat(), frame(&message).unwrap());
        assert!(written.len() > 1);
    }

    #[tokio::test]
    async fn a_failed_write_ends_the_send() {
        let fake = Fake {
            broken: true,
            ..Fake::default()
        };
        let mut framed = Framed::new(fake, notifications(vec![]));

        let failed = framed.send(b"hello").await.unwrap_err();

        assert!(matches!(failed, Error::WriteTimeout));
    }

    #[tokio::test]
    async fn notifications_split_anywhere_come_back_as_whole_messages() {
        let first: Vec<u8> = (0..40).collect();
        let second = b"second".to_vec();
        let bytes = [frame(&first).unwrap(), frame(&second).unwrap()].concat();
        // Cut where no message or piece boundary falls.
        let pieces = bytes.chunks(7).map(<[u8]>::to_vec).collect();
        let mut framed = Framed::new(Fake::default(), notifications(pieces));

        let wait = Duration::from_secs(1);
        assert_eq!(framed.receive(wait).await.unwrap(), first);
        assert_eq!(framed.receive(wait).await.unwrap(), second);
    }

    #[tokio::test]
    async fn a_link_that_ends_mid_message_is_dropped() {
        let bytes = frame(b"cut short").unwrap();
        let pieces = vec![bytes[..4].to_vec()];
        let mut framed = Framed::new(Fake::default(), notifications(pieces));

        let failed = framed.receive(Duration::from_secs(1)).await.unwrap_err();

        assert!(matches!(failed, Error::Dropped));
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_feeder_is_silent_after_the_wait() {
        // Held open and never written to: the feeder says nothing.
        let (_held, quiet) = mpsc::unbounded_channel::<Vec<u8>>();
        let quiet = stream::unfold(quiet, |mut quiet| async move {
            quiet.recv().await.map(|piece| (piece, quiet))
        });
        let mut framed = Framed::new(Fake::default(), Box::pin(quiet));

        let failed = framed.receive(Duration::from_secs(10)).await.unwrap_err();

        assert!(matches!(failed, Error::Silent));
    }
}
