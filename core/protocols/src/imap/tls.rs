// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TLS setup: TLS 1.2 and newer only, certificates always checked.
//!
//! Two ways to trust a server:
//! - **Roots** (the default): the certificate must chain to the bundled Mozilla roots
//!   (`webpki-roots`) or the platform store (`rustls-native-certs`) and match the host name.
//! - **Pinned**: the leaf certificate's SHA-256 must equal the fingerprint the caller passed in.
//!   Used for self-signed servers. On a mismatch the handshake fails; there is no fallback.
//!
//! Nothing here can turn verification off for a real connection. [`Trust::Capture`] exists only
//! for [`probe_certificate`](super::probe_certificate), which never sends any data.

use super::config::CertFingerprint;
use crate::types::BackendError;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{
    ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, OtherError,
    RootCertStore, SignatureScheme,
};

/// How the server's certificate is judged.
#[derive(Debug, Clone)]
pub(crate) enum Trust {
    /// Chain to the webpki or platform roots and match the host name.
    Roots,
    /// Exactly this leaf certificate.
    Pinned(CertFingerprint),
    /// Record the leaf fingerprint without trusting it. Only for the certificate probe, which
    /// closes the connection without sending anything.
    Capture(Arc<Mutex<Option<CertFingerprint>>>),
}

/// The pin did not match.
#[derive(Debug)]
struct PinMismatch;

impl std::fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("certificate fingerprint mismatch")
    }
}

impl std::error::Error for PinMismatch {}

/// Verifier for the pinned and capture modes. Signatures on the handshake are always verified
/// with the real algorithms, so a pin cannot be satisfied by replaying a public certificate.
#[derive(Debug)]
struct PinVerifier {
    mode: Trust,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let seen = CertFingerprint::of_der(end_entity.as_ref());
        match &self.mode {
            Trust::Pinned(pin) if *pin == seen => Ok(ServerCertVerified::assertion()),
            Trust::Capture(slot) => {
                if let Ok(mut guard) = slot.lock() {
                    *guard = Some(seen);
                }
                Ok(ServerCertVerified::assertion())
            }
            _ => Err(TlsError::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(PinMismatch)),
            ))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

static ROOTS: OnceCell<Arc<RootCertStore>> = OnceCell::const_new();

/// The root store: Mozilla's bundled roots plus whatever the platform trusts. Built once per
/// process. Platform roots that cannot be read are skipped (the bundled ones remain).
async fn roots() -> Arc<RootCertStore> {
    ROOTS
        .get_or_init(|| async {
            let built = tokio::task::spawn_blocking(|| {
                let mut store = RootCertStore::empty();
                store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let native = rustls_native_certs::load_native_certs();
                let _ = store.add_parsable_certificates(native.certs);
                store
            })
            .await;
            Arc::new(built.unwrap_or_else(|_| {
                let mut store = RootCertStore::empty();
                store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                store
            }))
        })
        .await
        .clone()
}

/// Builds the client configuration: TLS 1.3 and 1.2, ring crypto, no client certificate.
pub(crate) async fn client_config(trust: &Trust) -> Result<Arc<ClientConfig>, BackendError> {
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[
            &tokio_rustls::rustls::version::TLS13,
            &tokio_rustls::rustls::version::TLS12,
        ])
        .map_err(|_| BackendError::Permanent("TLS could not be configured".into()))?;
    let config = match trust {
        Trust::Roots => builder
            .with_root_certificates(roots().await)
            .with_no_client_auth(),
        Trust::Pinned(_) | Trust::Capture(_) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier {
                mode: trust.clone(),
                algorithms: signature_algorithms(&provider),
            }))
            .with_no_client_auth(),
    };
    Ok(Arc::new(config))
}

fn signature_algorithms(provider: &CryptoProvider) -> WebPkiSupportedAlgorithms {
    provider.signature_verification_algorithms
}
