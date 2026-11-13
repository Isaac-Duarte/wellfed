//! The server's identity: an Ed25519 public key it holds for its life, the
//! one its HTTPS certificate on the home network carries and its mDNS
//! announcement names. A client pins it when the owner adds the server and
//! refuses a server that later answers with another
//! ([server identity](../../../../docs/server/README.md#server-identity)).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use wellfed_model::Hex;

/// `GET /identity`: the server's identity key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ServerIdentity {
    /// The Ed25519 public key, as 64 lowercase hex digits.
    #[schema(value_type = String)]
    pub key: Hex<32>,
}

/// The mDNS TXT key naming the identity key, as 64 lowercase hex digits.
pub const TXT_KEY: &str = "key";

/// How a person compares `key` with what the server logs: its first 16
/// bytes as eight groups of four hex digits.
pub fn fingerprint(key: &[u8; 32]) -> String {
    let hex = Hex(*key).to_string();
    let groups: Vec<&str> = hex
        .as_bytes()
        .chunks(4)
        .take(8)
        .filter_map(|group| std::str::from_utf8(group).ok())
        .collect();
    groups.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_is_the_keys_first_sixteen_bytes_in_groups() {
        let mut key = [0; 32];
        key[0] = 0xab;
        key[1] = 0xcd;
        key[15] = 0x01;
        key[16] = 0xff;

        assert_eq!(fingerprint(&key), "abcd 0000 0000 0000 0000 0000 0000 0001");
    }
}
