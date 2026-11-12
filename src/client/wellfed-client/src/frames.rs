//! Frames out of a byte stream that may split or join them: the device
//! API's TCP stream and BLE setup's notifications carry the same framing
//! ([`wellfed_session::frame`]), each with its own limit.

use wellfed_session::frame::{self, PREFIX};

/// Reassembles frames of at most `max` bytes.
#[derive(Debug)]
pub(crate) struct Frames {
    buffer: Vec<u8>,
    max: usize,
}

/// A prefix announced more than the stream carries: the length it named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TooLong(pub usize);

impl Frames {
    pub(crate) const fn new(max: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame's bytes, if one has arrived.
    pub(crate) fn next(&mut self) -> Result<Option<Vec<u8>>, TooLong> {
        let Some(prefix) = self.buffer.first_chunk::<PREFIX>() else {
            return Ok(None);
        };
        let announced = usize::from(u16::from_le_bytes(*prefix));
        let len = frame::parse_prefix(*prefix, self.max).ok_or(TooLong(announced))?;
        if self.buffer.len() < PREFIX + len {
            return Ok(None);
        }

        let message = self.buffer[PREFIX..PREFIX + len].to_vec();
        self.buffer.drain(..PREFIX + len);
        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(messages: &[&[u8]]) -> Vec<u8> {
        let mut stream = Vec::new();
        for message in messages {
            stream.extend(frame::prefix(message.len(), 300).unwrap());
            stream.extend_from_slice(message);
        }
        stream
    }

    fn drain(frames: &mut Frames) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| frames.next().unwrap()).collect()
    }

    /// However the stream is cut, into up to three pieces at any points,
    /// the same frames come out in order.
    #[test]
    fn any_split_of_the_stream_yields_the_same_frames() {
        let messages: [&[u8]; 4] = [b"one", b"", &[7; 260], b"four"];
        let stream = framed(&messages);
        let whole: Vec<Vec<u8>> = messages.iter().map(|m| m.to_vec()).collect();

        for first in 0..=stream.len() {
            for second in (first..=stream.len()).step_by(7) {
                let mut frames = Frames::new(300);
                let mut out = Vec::new();
                for piece in [&stream[..first], &stream[first..second], &stream[second..]] {
                    frames.push(piece);
                    out.extend(drain(&mut frames));
                }
                assert_eq!(out, whole, "cut at {first} and {second}");
            }
        }
    }

    #[test]
    fn a_frame_past_the_limit_is_refused_by_its_length() {
        let mut frames = Frames::new(4);
        frames.push(&[5, 0, 1, 2, 3, 4, 5]);
        assert_eq!(frames.next(), Err(TooLong(5)));
    }
}
