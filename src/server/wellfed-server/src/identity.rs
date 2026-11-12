//! The server's identity key: Ed25519, derived from the sealing key, so it
//! needs no secret file of its own and stays the server's for as long as
//! its key does. A server restored with its key keeps its identity; one
//! that lost the key cannot start anyway ([`crate::key`]). The derivation
//! is one-way: the public key, which the LAN hears, says nothing of the
//! sealing key.
//!
//! The key signs the server's own HTTPS on the home network: its
//! certificate is self-signed with it, and a TLS 1.3 handshake proves the
//! server holds it, which is what the app pins
//! ([server identity](../../../../docs/server/README.md#server-identity)).

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// The DER of a PKCS#8 v1 Ed25519 private key up to its 32-byte seed
/// (RFC 8410, section 7), which is all the key file format adds.
const PKCS8_ED25519: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// The name the certificate gives the server. Clients check the key, never
/// the name: the server is reached by whatever address it has.
const SUBJECT: &str = "Wellfed server";

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("make the HTTPS certificate")]
    Certificate(#[from] rcgen::Error),
    #[error("set up HTTPS")]
    Tls(#[from] rustls::Error),
}

/// The server's identity key.
#[derive(Clone)]
pub struct Identity {
    signing: SigningKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The public key only: the rest is a secret.
        f.debug_struct("Identity")
            .field("key", &wellfed_api::fingerprint(&self.public()))
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// The identity that `sealing`, the server's sealing key, gives.
    pub fn derive(sealing: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(&wellfed_service::identity_seed(sealing)),
        }
    }

    /// The public key, as clients pin it.
    pub fn public(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// The server's HTTPS: TLS 1.3 only, with a certificate this key
    /// signed for itself.
    pub fn tls(&self) -> Result<Arc<ServerConfig>, IdentityError> {
        let (certificate, key) = self.certificate()?;
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }

    /// A certificate naming this key, self-signed with it, and the key as
    /// rustls takes it.
    fn certificate(
        &self,
    ) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), IdentityError> {
        let mut pkcs8 = PKCS8_ED25519.to_vec();
        pkcs8.extend_from_slice(self.signing.as_bytes());

        let pair = rcgen::KeyPair::try_from(pkcs8.as_slice())?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new())?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, SUBJECT);
        let certificate = params.self_signed(&pair)?;

        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8));
        Ok((certificate.der().clone(), key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_follows_the_sealing_key_and_never_shows_it() {
        let one = Identity::derive(&[1; 32]);
        let again = Identity::derive(&[1; 32]);
        let other = Identity::derive(&[2; 32]);

        assert_eq!(one.public(), again.public());
        assert_ne!(one.public(), other.public());
        assert_ne!(one.signing.to_bytes(), [1; 32]);
        assert!(
            !format!("{one:?}").contains(&wellfed_api::Hex(one.signing.to_bytes()).to_string())
        );
    }

    #[test]
    fn its_https_certificate_carries_its_key() {
        let identity = Identity::derive(&[3; 32]);

        let (certificate, _) = identity.certificate().unwrap();

        let key = identity.public();
        assert!(certificate.windows(32).any(|window| window == key));
        identity.tls().unwrap();
    }
}
