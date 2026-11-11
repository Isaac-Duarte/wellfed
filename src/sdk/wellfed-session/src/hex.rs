//! Strict lowercase hex, the only form the firmware's text fields accept.
//!
//! Keys, MACs, nonces and Wi-Fi credentials all arrive as lowercase hex
//! digits; an uppercase digit or any other byte is refused rather than
//! normalized, so a request has exactly one accepted spelling.
//!
//! This is the workspace's one hex codec, for hosts too. A host writes
//! lowercase ([`Lower`], whose `to_string` is the owned form) and reads
//! either case only where the text is a person's or another program's (a
//! key on a command line, an eFuse dump, a MAC address), through the
//! `_either_case` decoders, which the firmware never calls.

/// One lowercase hex digit's value.
#[inline]
pub const fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Every byte is a lowercase hex digit (true for an empty input).
pub fn is_lower(text: &[u8]) -> bool {
    text.iter().all(|&b| nibble(b).is_some())
}

/// One hex digit's value, either case.
#[inline]
pub const fn nibble_either_case(c: u8) -> Option<u8> {
    nibble(c.to_ascii_lowercase())
}

/// Decode `hex` into the start of `out` and return the byte count; `None`
/// for an odd length, more bytes than `out` holds, or a non-digit. `out` may
/// be partly written on failure.
pub fn decode(hex: &[u8], out: &mut [u8]) -> Option<usize> {
    decode_digits(hex, out, nibble)
}

/// Exactly `2 * N` lowercase hex digits as `N` bytes.
pub fn decode_array<const N: usize>(hex: &[u8]) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    (hex.len() == 2 * N && decode(hex, &mut out).is_some()).then_some(out)
}

/// [`decode`], taking uppercase digits too.
pub fn decode_either_case(hex: &[u8], out: &mut [u8]) -> Option<usize> {
    decode_digits(hex, out, nibble_either_case)
}

/// [`decode_array`], taking uppercase digits too.
pub fn decode_array_either_case<const N: usize>(hex: &[u8]) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    (hex.len() == 2 * N && decode_either_case(hex, &mut out).is_some()).then_some(out)
}

fn decode_digits(hex: &[u8], out: &mut [u8], digit: impl Fn(u8) -> Option<u8>) -> Option<usize> {
    if !hex.len().is_multiple_of(2) || hex.len() / 2 > out.len() {
        return None;
    }

    for (byte, pair) in out.iter_mut().zip(hex.as_chunks::<2>().0) {
        *byte = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(hex.len() / 2)
}

/// Bytes as lowercase hex when formatted, for record fields.
#[derive(Clone, Copy, Debug)]
pub struct Lower<'a>(pub &'a [u8]);

impl core::fmt::Display for Lower<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// Bytes as lowercase hex into the start of `out`; `None` when it is
/// shorter than `2 * bytes.len()`.
pub fn encode<'a>(bytes: &[u8], out: &'a mut [u8]) -> Option<&'a [u8]> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let out = out.get_mut(..2 * bytes.len())?;
    for (pair, byte) in out.as_chunks_mut::<2>().0.iter_mut().zip(bytes) {
        *pair = [
            DIGITS[usize::from(byte >> 4)],
            DIGITS[usize::from(byte & 0xf)],
        ];
    }
    Some(out)
}
