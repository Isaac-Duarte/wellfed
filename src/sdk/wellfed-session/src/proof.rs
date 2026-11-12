//! Proof that a client holds the secret of its static key, answered to a
//! challenge from the owner's server. The server records a phone's key on
//! a feeder under the token of the phone's login; a key a logout left with
//! no token is put under a new token only by a phone that proves it holds
//! that key, so another token of the same account (a stolen one) cannot
//! take the key and then have it revoked with itself.
//!
//! The server makes an ephemeral key pair and a nonce. The client answers
//! with an HMAC over the nonce, its token's id and its public key, keyed by
//! a key derived from the Diffie-Hellman of its static secret and the
//! ephemeral public key; the server derives the same key from the ephemeral
//! secret and the client's public key. Only the holder of either secret
//! can make the MAC, and the server keeps its own: the ephemeral key serves
//! one challenge. Nothing here touches the device API's handshakes.

use crate::mac::{hmac_sha256, same};
use crate::noise::{Error, KEY_LEN, Keypair};

/// A challenge's nonce, bytes.
pub const NONCE_LEN: usize = 32;

/// A token id, bytes (a UUID).
pub const TOKEN_LEN: usize = 16;

/// Separates this MAC's key from every other use of the same DH.
const LABEL: &[u8] = b"wellfed client key proof v1";

/// The MAC `holder` answers a challenge with: `challenge` is the server's
/// ephemeral public key, `nonce` its nonce, and `token` the id of the token
/// the claim goes under. Fails on a low-order `challenge`.
pub fn prove(
    holder: &Keypair,
    challenge: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    token: &[u8; TOKEN_LEN],
) -> Result<[u8; 32], Error> {
    let shared = holder.dh(challenge)?;
    Ok(mac(&shared, nonce, token, holder.public()))
}

/// Whether `proof` is what the holder of `key` answers `challenge`'s
/// ephemeral key pair, nonce and `token` with. A low-order `key` proves
/// nothing.
pub fn verify(
    challenge: &Keypair,
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    token: &[u8; TOKEN_LEN],
    proof: &[u8; 32],
) -> bool {
    let Ok(shared) = challenge.dh(key) else {
        return false;
    };
    same(&mac(&shared, nonce, token, key), proof)
}

fn mac(
    shared: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    token: &[u8; TOKEN_LEN],
    key: &[u8; KEY_LEN],
) -> [u8; 32] {
    let derived = hmac_sha256(shared, &[LABEL]);
    hmac_sha256(&derived, &[nonce, token, key])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: [u8; NONCE_LEN] = [7; NONCE_LEN];
    const TOKEN: [u8; TOKEN_LEN] = [9; TOKEN_LEN];

    fn pair(n: u8) -> Keypair {
        Keypair::from_secret([n; KEY_LEN])
    }

    #[test]
    fn the_holders_proof_verifies_and_nothing_else_does() {
        let (phone, challenge) = (pair(1), pair(2));
        let proof = prove(&phone, challenge.public(), &NONCE, &TOKEN).unwrap();
        assert!(verify(&challenge, phone.public(), &NONCE, &TOKEN, &proof));

        let other = prove(&pair(3), challenge.public(), &NONCE, &TOKEN).unwrap();
        assert!(!verify(&challenge, phone.public(), &NONCE, &TOKEN, &other));
        assert!(!verify(
            &challenge,
            pair(3).public(),
            &NONCE,
            &TOKEN,
            &proof
        ));
        assert!(!verify(&pair(4), phone.public(), &NONCE, &TOKEN, &proof));
        assert!(!verify(
            &challenge,
            phone.public(),
            &[8; NONCE_LEN],
            &TOKEN,
            &proof
        ));
        assert!(!verify(
            &challenge,
            phone.public(),
            &NONCE,
            &[0; TOKEN_LEN],
            &proof
        ));
    }

    #[test]
    fn a_low_order_key_proves_nothing() {
        let zero = [0; KEY_LEN];
        assert_eq!(prove(&pair(1), &zero, &NONCE, &TOKEN), Err(Error::WeakKey));
        assert!(!verify(&pair(2), &zero, &NONCE, &TOKEN, &[0; 32]));
    }
}
