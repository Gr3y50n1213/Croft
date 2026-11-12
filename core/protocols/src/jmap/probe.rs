// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reads the certificate fingerprint of a JMAP server, for the account-setup screen.

use super::config::JmapConfig;
use super::tls::{self, Trust};
use crate::imap::CertFingerprint;
use crate::types::BackendError;
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;

/// Connects to the server named by `config.session_url`, completes the TLS handshake **without
/// trusting the certificate**, reads the leaf certificate's SHA-256 fingerprint, and disconnects.
/// No credential and no request is ever sent.
///
/// This is for the account-setup screen: when a server's certificate does not validate, the person
/// is shown this fingerprint to compare with the one the server's owner published, and may then
/// pin it with [`JmapConfig::with_pinned_cert`]. Calling it trusts nothing by itself. Any
/// `pinned_cert` already in `config` is ignored. Returns `Permanent` for a plain `http` address,
/// which has no certificate.
pub async fn probe_certificate(config: &JmapConfig) -> Result<CertFingerprint, BackendError> {
    let url = config.validate()?;
    if !url.secure {
        return Err(BackendError::Permanent(
            "an http address has no certificate".into(),
        ));
    }
    let unreachable = || BackendError::Transient("could not connect to the server".into());
    let addrs: Vec<_> = timeout(
        config.timeout,
        tokio::net::lookup_host((url.host.as_str(), url.port)),
    )
    .await
    .map_err(|_| unreachable())?
    .map_err(|_| unreachable())?
    .take(4)
    .collect();
    // T26: the process's address policy (the Companion refuses its own network).
    let addrs = crate::netpolicy::filter(addrs);
    if addrs.is_empty() {
        return Err(BackendError::Permanent(crate::netpolicy::REFUSED.into()));
    }
    let per_address = (config.timeout / 3).max(std::time::Duration::from_millis(200));
    let mut tcp = None;
    for addr in addrs {
        if let Ok(Ok(s)) = timeout(per_address, TcpStream::connect(addr)).await {
            tcp = Some(s);
            break;
        }
    }
    let tcp = tcp.ok_or_else(unreachable)?;
    let seen = Arc::new(Mutex::new(None));
    let connector = TlsConnector::from(tls::client_config(&Trust::Capture(seen.clone())).await?);
    let name = ServerName::try_from(url.host.clone())
        .map_err(|_| BackendError::Permanent("the server name is not valid".into()))?;
    // The handshake is all that is needed; whether it completes cleanly does not matter once the
    // verifier has seen the certificate.
    let _ = timeout(config.timeout, connector.connect(name, tcp)).await;
    let fingerprint = seen.lock().ok().and_then(|g| *g);
    fingerprint
        .ok_or_else(|| BackendError::Transient("the server did not present a certificate".into()))
}

/// Checks the server's certificate **without sending anything**: one TLS handshake with the
/// account's trust (the pin when `config` has one, else the webpki and platform roots), no HTTP
/// request and no credential. Used before a sign-in check, so that a password is only ever sent
/// to a certificate that validates or that the person confirmed (T22). An `http` address (allowed
/// for loopback only) has no certificate and passes.
pub async fn check_certificate(config: &JmapConfig) -> Result<(), crate::imap::CertificateProblem> {
    use crate::imap::CertificateProblem as P;
    let url = config.validate().map_err(|_| P::Other)?;
    if !url.secure {
        return Ok(());
    }
    let addrs: Vec<_> = match timeout(
        config.timeout,
        tokio::net::lookup_host((url.host.as_str(), url.port)),
    )
    .await
    {
        Ok(Ok(a)) => a.take(4).collect(),
        _ => return Err(P::CannotConnect),
    };
    // T26: the process's address policy (the Companion refuses its own network).
    let addrs = crate::netpolicy::filter(addrs);
    if addrs.is_empty() {
        return Err(P::CannotConnect);
    }
    let per_address = (config.timeout / 3).max(std::time::Duration::from_millis(200));
    let mut tcp = None;
    for addr in addrs {
        if let Ok(Ok(s)) = timeout(per_address, TcpStream::connect(addr)).await {
            tcp = Some(s);
            break;
        }
    }
    let tcp = tcp.ok_or(P::CannotConnect)?;
    let trust = match config.pinned_cert {
        Some(pin) => Trust::Pinned(pin),
        None => Trust::Roots,
    };
    let connector = TlsConnector::from(tls::client_config(&trust).await.map_err(|_| P::Other)?);
    let name = ServerName::try_from(url.host.clone()).map_err(|_| P::Other)?;
    let failed = match timeout(config.timeout, connector.connect(name, tcp)).await {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(e)) => e,
        Err(_) => return Err(P::CannotConnect),
    };
    let Some(tls_error) = failed
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<tokio_rustls::rustls::Error>())
    else {
        // Not a certificate problem: the connection broke.
        return Err(P::CannotConnect);
    };
    // The certificate the person would be shown (a second handshake that trusts nothing).
    let fingerprint = probe_certificate(config).await.ok();
    if tls::is_pin_mismatch(tls_error) {
        return Err(match fingerprint {
            Some(fingerprint) => P::PinMismatch { fingerprint },
            None => P::Other,
        });
    }
    Err(P::from_tls(tls_error, fingerprint))
}
