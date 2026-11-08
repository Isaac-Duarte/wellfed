//! What lets a message grow without stranding an older reader
//! ([device API](../../../../docs/firmware/device-api.md#evolution)).
//!
//! - [`Lenient`] carries a unit-only enum ([`unit_enum!`]) inside an answer.
//!   It encodes as the enum does, and a reader of an older build decodes an
//!   index it does not know as [`Lenient::Unknown`] rather than failing the
//!   whole answer. Such an enum only ever gains unit variants, which is all
//!   the macro takes.
//! - [`Tail`] ends a struct that will grow: a length-prefixed run of bytes
//!   that this build leaves empty and a later build fills with its new
//!   fields, encoded as a struct of their own. An older reader skips it
//!   whole. A device refuses a request whose tail it cannot read
//!   ([`Tail::is_empty`]), since it would drop what the client asked for.

use core::fmt;
use core::marker::PhantomData;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A unit-only enum that may gain variants at its end: [`Lenient`] reads
/// its index. Implemented by [`unit_enum!`] alone.
pub trait Unit: Copy + Serialize + 'static {
    /// The enum's serde name, which its trace is filed under.
    const NAME: &'static str;
    /// Its variants' names, in index order.
    const VARIANTS: &'static [&'static str];
    /// The name a [`Lenient`] of it is traced under.
    const LENIENT: &'static str;

    /// Its index, as postcard encodes it.
    fn index(self) -> u32;
    /// The variant at `index`, if this build knows it.
    fn from_index(index: u32) -> Option<Self>;
}

/// Declares a unit-only enum with its [`Unit`] impl: only unit variants
/// are accepted, so the enum can only grow by them.
macro_rules! unit_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                $variant
            ),*
        }

        impl $crate::grow::Unit for $name {
            const NAME: &'static str = stringify!($name);
            const VARIANTS: &'static [&'static str] = &[$(stringify!($variant)),*];
            const LENIENT: &'static str = concat!("Lenient", stringify!($name));

            fn index(self) -> u32 {
                self as u32
            }

            fn from_index(index: u32) -> Option<Self> {
                const ALL: &[$name] = &[$($name::$variant),*];
                ALL.get(usize::try_from(index).ok()?).copied()
            }
        }
    };
}

/// A unit-only enum as an answer carries it: what an older reader makes of
/// a variant a newer build added. A device only ever sends
/// [`Self::Known`]; the encoding is the enum's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lenient<T> {
    Known(T),
    /// A variant this build does not know, by its index: saturated to a
    /// byte, which keeps the value as small as the device's own enums (no
    /// unit enum here comes near 256 variants).
    Unknown(u8),
}

impl<T> From<T> for Lenient<T> {
    fn from(value: T) -> Self {
        Self::Known(value)
    }
}

impl<T: Default> Default for Lenient<T> {
    fn default() -> Self {
        Self::Known(T::default())
    }
}

impl<T: Copy> Lenient<T> {
    /// The variant, if this build knows it.
    pub const fn known(self) -> Option<T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unknown(_) => None,
        }
    }
}

// A unit variant and a `u32` are one varint in postcard, so `Lenient`
// reads and writes the index as a `u32` behind a newtype named for its enum.
// Read as an enum, a tracer could not tell its variants apart (they decode to
// one `Lenient::Known` each); read this way, the trace still names the enum,
// whose own trace holds its variants.
impl<T: Unit> Serialize for Lenient<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let index = match self {
            Self::Known(value) => value.index(),
            Self::Unknown(index) => u32::from(*index),
        };
        serializer.serialize_newtype_struct(T::LENIENT, &index)
    }
}

impl<'de, T: Unit> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Index<T>(PhantomData<T>);

        impl<'de, T: Unit> Visitor<'de> for Index<T> {
            type Value = Lenient<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a unit variant's index")
            }

            fn visit_newtype_struct<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                let index = u32::deserialize(deserializer)?;
                Ok(T::from_index(index).map_or_else(
                    || Lenient::Unknown(u8::try_from(index).unwrap_or(u8::MAX)),
                    Lenient::Known,
                ))
            }
        }

        deserializer.deserialize_newtype_struct(T::LENIENT, Index(PhantomData))
    }
}

/// The end of a struct that will grow: see the [module](self). This build
/// writes it empty and reads only how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tail {
    len: usize,
}

impl Tail {
    /// What this build writes.
    pub const EMPTY: Self = Self { len: 0 };

    /// Whether a later build's fields came in it. A device refuses a
    /// request that carries any, since it cannot do what they ask.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Serialize for Tail {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Nothing of this build goes in a tail: one that was read is
        // never written on, so a client cannot pass a newer build's fields
        // back as though it had set them.
        serializer.serialize_bytes(&[])
    }
}

impl<'de> Deserialize<'de> for Tail {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Skip;

        impl Visitor<'_> for Skip {
            type Value = Tail;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a struct's extension bytes")
            }

            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Tail, E> {
                Ok(Tail { len: bytes.len() })
            }
        }

        deserializer.deserialize_bytes(Skip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, encode};

    unit_enum! {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        enum Old {
            A,
            B,
        }
    }

    unit_enum! {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        enum New {
            A,
            B,
            C,
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Reading<T> {
        before: u8,
        level: T,
        after: u8,
    }

    /// A variant added after an older reader's build reads as unknown, by
    /// its index, and the fields around it still read.
    #[test]
    fn a_newer_variant_reads_as_unknown_and_the_rest_reads_on() {
        let mut out = [0; 8];
        let len = encode(
            &Reading {
                before: 1,
                level: New::C,
                after: 2,
            },
            &mut out,
        )
        .unwrap();

        let read: Reading<Lenient<Old>> = decode(&out[..len]).unwrap();

        assert_eq!(
            read,
            Reading {
                before: 1,
                level: Lenient::Unknown(2),
                after: 2
            }
        );
    }

    /// A known variant encodes and reads as the plain enum does.
    #[test]
    fn a_known_variant_is_the_enum_s_own_bytes() {
        let mut plain = [0; 4];
        let mut lenient = [0; 4];
        let plain_len = encode(&New::B, &mut plain).unwrap();
        let lenient_len = encode(&Lenient::Known(New::B), &mut lenient).unwrap();

        assert_eq!(plain[..plain_len], lenient[..lenient_len]);
        assert_eq!(
            decode::<Lenient<New>>(&plain[..plain_len]),
            Ok(Lenient::Known(New::B))
        );
    }

    /// A later build's tail is skipped whole, and this build writes none.
    #[test]
    fn a_tail_is_skipped_by_its_length() {
        #[derive(Serialize)]
        struct Grown {
            a: u8,
            tail: &'static [u8],
            b: u8,
        }
        #[derive(Debug, PartialEq, Deserialize)]
        struct Known {
            a: u8,
            tail: Tail,
            b: u8,
        }
        let mut out = [0; 16];
        let len = encode(
            &Grown {
                a: 1,
                tail: &[9, 9, 9],
                b: 2,
            },
            &mut out,
        )
        .unwrap();

        let read: Known = decode(&out[..len]).unwrap();

        assert_eq!((read.a, read.b), (1, 2));
        assert!(!read.tail.is_empty());
        let len = encode(&Tail::EMPTY, &mut out).unwrap();
        assert_eq!(out[..len], [0]);
    }
}
