//! How a client checks the server it reaches over HTTPS.
//!
//! A server on the home network serves its own HTTPS with a certificate it
//! signed itself with its identity key, an Ed25519 key it holds for life
//! ([server identity]). No authority vouches for it, so the client pins the
//! key: [`Trust::Pinned`] accepts a TLS 1.3 handshake only when its
//! signature verifies under the pinned key. The certificate's own contents
//! count for nothing, its name and dates included: the server is reached by
//! whatever address it has, and what proves it is the handshake signed by
//! the key. A server that answers with another key is
//! [`Error::KeyChanged`](crate::Error::KeyChanged), before a byte of the
//! request is sent.
//!
//! [`identify`] is the first contact with a server whose key the client
//! has not been told (one whose address was typed): it accepts any key that
//! signs the handshake, and reports which, for the caller to pin.
//!
//! A server reached by a name with a public certificate, behind a reverse
//! proxy, is [`Trust::Platform`]: the system's authorities vouch for the
//! name, and its key is read from `GET /identity`.
//!
//! [server identity]: ../../../../docs/server/README.md#server-identity

use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signature, VerifyingKey};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// How a client checks the server's HTTPS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trust {
    /// The system's certificate authorities, as for any site: a server
    /// behind a reverse proxy with a public certificate.
    #[default]
    Platform,
    /// The server's own HTTPS: the handshake must be signed by this
    /// identity key, whatever its certificate says.
    Pinned([u8; 32]),
}

/// The DER an Ed25519 key's `SubjectPublicKeyInfo` starts with, before its
/// 32 bytes (RFC 8410, section 4).
const ED25519_SPKI: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// The handshake was signed by another key than the pinned one: carried
/// through rustls and reqwest to [`crate::Error::KeyChanged`].
#[derive(Debug, thiserror::Error)]
#[error("the server's identity key is not the one this client pinned")]
pub(crate) struct KeyMismatch {
    /// The key the server's certificate names, when it names one.
    pub(crate) offered: Option<[u8; 32]>,
}

/// The server proved no Ed25519 key: not a Wellfed server's own HTTPS.
#[derive(Debug, thiserror::Error)]
#[error("the server's certificate names no Ed25519 identity key")]
pub(crate) struct NoIdentity;

/// The Ed25519 key `certificate` names: the 32 bytes after the first
/// Ed25519 `SubjectPublicKeyInfo` header in its DER. A key that only looks
/// like one is harmless, since a key is believed only once the handshake
/// verifies under it.
fn named_key(certificate: &[u8]) -> Option<[u8; 32]> {
    let at = certificate
        .windows(ED25519_SPKI.len())
        .position(|window| window == ED25519_SPKI)?;
    let start = at + ED25519_SPKI.len();
    certificate.get(start..start + 32)?.try_into().ok()
}

/// Whether `signature` over `message` verifies under `key`, strictly.
fn signed_by(key: &[u8; 32], message: &[u8], signature: &DigitallySignedStruct) -> bool {
    if signature.scheme != SignatureScheme::ED25519 {
        return false;
    }
    let Ok(key) = VerifyingKey::from_bytes(key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature.signature()) else {
        return false;
    };
    key.verify_strict(message, &signature).is_ok()
}

fn mismatch(offered: Option<[u8; 32]>) -> rustls::Error {
    rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
        Arc::new(KeyMismatch { offered }),
    )))
}

fn no_identity() -> rustls::Error {
    rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
        Arc::new(NoIdentity),
    )))
}

/// What the verifier holds the handshake to.
#[derive(Debug)]
enum Expect {
    /// This key, and no other.
    Key([u8; 32]),
    /// Any key the handshake proves, kept here for the caller.
    FirstUse(Mutex<Option<[u8; 32]>>),
}

/// Checks a TLS 1.3 handshake against an identity key, never against an
/// authority.
#[derive(Debug)]
pub(crate) struct KeyVerifier {
    expect: Expect,
}

impl KeyVerifier {
    pub(crate) fn pinned(key: [u8; 32]) -> Self {
        Self {
            expect: Expect::Key(key),
        }
    }

    pub(crate) fn first_use() -> Self {
        Self {
            expect: Expect::FirstUse(Mutex::new(None)),
        }
    }

    /// The key a first-use handshake proved.
    pub(crate) fn proved(&self) -> Option<[u8; 32]> {
        match &self.expect {
            Expect::Key(_) => None,
            Expect::FirstUse(proved) => proved.lock().ok().and_then(|proved| *proved),
        }
    }
}

impl ServerCertVerifier for KeyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Refused early when the certificate names another key; one naming
        // the pinned key still has to sign the handshake with it.
        let named = named_key(end_entity);
        match (&self.expect, named) {
            (_, None) => Err(no_identity()),
            (Expect::Key(pinned), Some(named)) if named != *pinned => Err(mismatch(Some(named))),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Never asked: this client speaks TLS 1.3 alone, as the server's
        // own HTTPS does.
        Err(rustls::Error::General("TLS 1.2 is not spoken".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        match &self.expect {
            Expect::Key(pinned) => {
                if signed_by(pinned, message, dss) {
                    Ok(HandshakeSignatureValid::assertion())
                } else {
                    Err(mismatch(named_key(cert)))
                }
            }
            Expect::FirstUse(proved) => {
                let Some(named) = named_key(cert) else {
                    return Err(no_identity());
                };
                if !signed_by(&named, message, dss) {
                    return Err(rustls::Error::InvalidCertificate(
                        rustls::CertificateError::BadSignature,
                    ));
                }
                if let Ok(mut proved) = proved.lock() {
                    *proved = Some(named);
                }
                Ok(HandshakeSignatureValid::assertion())
            }
        }
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// The TLS a client checking `verifier` speaks: TLS 1.3 with HTTP/1.1,
/// which is all the server's own HTTPS serves.
pub(crate) fn config(verifier: Arc<KeyVerifier>) -> Result<rustls::ClientConfig, rustls::Error> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// The `T` a failed request carries somewhere down its sources, rustls's
/// own boxed certificate error included.
pub(crate) fn carried<'a, T>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a T>
where
    T: std::error::Error + 'static,
{
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(found) = error.downcast_ref::<T>() {
            return Some(found);
        }
        if let Some(rustls::Error::InvalidCertificate(rustls::CertificateError::Other(other))) =
            error.downcast_ref::<rustls::Error>()
            && let Some(found) = other.0.downcast_ref::<T>()
        {
            return Some(found);
        }
        // An `io::Error`'s `source` skips the error it wraps.
        next = match error.downcast_ref::<std::io::Error>() {
            Some(io) => io
                .get_ref()
                .map(|inner| inner as &(dyn std::error::Error + 'static)),
            None => error.source(),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_key_follows_the_ed25519_header() {
        let mut der = vec![0x30, 0x82];
        der.extend_from_slice(&ED25519_SPKI);
        der.extend_from_slice(&[7; 32]);
        der.push(0xa3);

        assert_eq!(named_key(&der), Some([7; 32]));
        assert_eq!(named_key(&der[..der.len() - 2]), None, "cut short");
        assert_eq!(named_key(&[0x30, 0x00]), None);
    }
}
