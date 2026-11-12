// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bring-your-own OAuth 2 (T21): Authorization Code with PKCE (RFC 7636, `S256`) for Gmail and
//! Outlook, with a client the person registered themselves. Croft ships no client id and no
//! secret.
//!
//! This module is the pure part: the provider table, the PKCE pair, the authorisation URL, the
//! check of the redirect the browser comes back with, the token request bodies and the reading of
//! the token endpoint's answer. Sending the requests is [`token`]'s job, and only to the hosts of
//! the provider table.
//!
//! Nothing here prints a token, a code, a verifier or a secret: every such value is held in a
//! [`Zeroizing`] string and the `Debug` output of each type says only what kind of value it is.

pub mod loopback;
pub mod token;

use base64::Engine as _;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::types::BackendError;

/// The providers Croft knows how to sign in to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OAuthProvider {
    /// Gmail.
    Google,
    /// Outlook and Microsoft 365.
    Microsoft,
}

impl OAuthProvider {
    /// The provider's stable key (settings, FFI).
    pub fn key(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    /// The provider for a key.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }

    /// The authorisation endpoint (opened in the browser).
    pub fn authorize_url(self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        }
    }

    /// The token endpoint (the core sends the code and the refresh token here, and nowhere else).
    pub fn token_url(self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        }
    }

    /// The scopes asked for: mail over IMAP and SMTP, and a refresh token.
    pub fn scopes(self) -> &'static [&'static str] {
        match self {
            // Google's one scope for IMAP and SMTP; installed-app clients get a refresh token
            // without asking for one.
            Self::Google => &["https://mail.google.com/"],
            Self::Microsoft => &[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "https://outlook.office.com/SMTP.Send",
                "offline_access",
            ],
        }
    }

    /// The only host the core sends anything to for this provider.
    pub fn token_host(self) -> &'static str {
        match self {
            Self::Google => "oauth2.googleapis.com",
            Self::Microsoft => "login.microsoftonline.com",
        }
    }
}

/// Why a sign-in through the provider's page did not work. Each is a plain message on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OAuthError {
    /// The browser came back with another flow's `state` (an old tab, or a forged link).
    StateMismatch,
    /// The redirect had neither a code nor an error.
    MissingCode,
    /// The person said no on the provider's page.
    AccessDenied,
    /// The redirect went somewhere else than the redirect this flow asked for.
    WrongRedirect,
    /// The provider refused the request: the client id, the redirect or the client type does
    /// not match what was registered. The provider's short error code (never its description,
    /// which can quote the request).
    Refused(String),
    /// The refresh token was revoked or expired (`invalid_grant`): sign in again.
    InvalidGrant,
    /// The network or the provider failed for now.
    Transient,
    /// The provider's answer could not be read.
    Malformed,
}

impl From<OAuthError> for BackendError {
    fn from(e: OAuthError) -> Self {
        match e {
            OAuthError::InvalidGrant => BackendError::Auth("the sign-in has expired".into()),
            OAuthError::Transient => {
                BackendError::Transient("the sign-in service did not answer".into())
            }
            OAuthError::AccessDenied => BackendError::Auth("sign-in was cancelled".into()),
            other => BackendError::Permanent(format!("sign-in failed: {}", other.code())),
        }
    }
}

impl OAuthError {
    /// A short stable code (FFI, tests). Never holds a token or a URL.
    pub fn code(&self) -> &str {
        match self {
            Self::StateMismatch => "state_mismatch",
            Self::MissingCode => "missing_code",
            Self::AccessDenied => "access_denied",
            Self::WrongRedirect => "wrong_redirect",
            Self::Refused(c) => c,
            Self::InvalidGrant => "invalid_grant",
            Self::Transient => "transient",
            Self::Malformed => "malformed",
        }
    }
}

/// The PKCE pair of one flow: the verifier stays in the core; the challenge goes in the URL.
pub struct Pkce {
    /// The `code_verifier` (43 characters of the unreserved set).
    pub verifier: Zeroizing<String>,
    /// `BASE64URL(SHA256(verifier))`, no padding.
    pub challenge: String,
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pkce(***)")
    }
}

impl Pkce {
    /// The pair for a given verifier.
    pub fn from_verifier(verifier: &str) -> Self {
        Self {
            verifier: Zeroizing::new(verifier.to_owned()),
            challenge: challenge(verifier),
        }
    }

    /// A fresh pair from 32 random bytes.
    pub fn generate() -> Result<Self, BackendError> {
        Ok(Self::from_verifier(&random_token()?))
    }
}

/// `BASE64URL-ENCODE(SHA256(ASCII(verifier)))` (RFC 7636 section 4.2).
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 32 random bytes, base64url without padding (43 characters): a PKCE verifier or a `state`.
pub fn random_token() -> Result<String, BackendError> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes[..])
        .map_err(|_| BackendError::Permanent("no random numbers".into()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes[..]))
}

/// Percent-encodes everything but the unreserved characters (RFC 3986), for a query value.
pub fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn form_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = value.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// What the authorisation URL is made of.
pub struct AuthRequest<'a> {
    /// The provider.
    pub provider: OAuthProvider,
    /// The person's own client id (not a secret).
    pub client_id: &'a str,
    /// Where the browser comes back to.
    pub redirect_uri: &'a str,
    /// The flow's random `state`.
    pub state: &'a str,
    /// The PKCE challenge.
    pub challenge: &'a str,
    /// The address, to preselect the account on the provider's page.
    pub login_hint: Option<&'a str>,
}

/// The URL the browser opens: code flow, the provider's scopes, `S256`, no secret.
pub fn authorization_url(r: &AuthRequest<'_>) -> String {
    let scope = r.provider.scopes().join(" ");
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        r.provider.authorize_url(),
        form_encode(r.client_id),
        form_encode(r.redirect_uri),
        form_encode(&scope),
        form_encode(r.state),
        form_encode(r.challenge),
    );
    if let Some(hint) = r.login_hint.filter(|h| !h.is_empty()) {
        url.push_str("&login_hint=");
        url.push_str(&form_encode(hint));
    }
    url
}

/// Reads the redirect the browser came back with: it must be this flow's redirect, carry this
/// flow's `state`, and a code. A provider's `error` (the person said no, a refused client)
/// becomes its typed error. Returns the code.
pub fn parse_redirect(
    url: &str,
    redirect_uri: &str,
    expected_state: &str,
) -> Result<Zeroizing<String>, OAuthError> {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    // A fragment is never part of the answer (code flow answers in the query).
    let query = query.split('#').next().unwrap_or("");
    if base.trim_end_matches('/') != redirect_uri.trim_end_matches('/') {
        return Err(OAuthError::WrongRedirect);
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = form_decode(v).ok_or(OAuthError::Malformed)?;
        match k {
            "code" => code = Some(Zeroizing::new(v)),
            "state" => state = Some(v),
            "error" => error = Some(v),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(OAuthError::StateMismatch);
    }
    if let Some(e) = error {
        return Err(match e.as_str() {
            "access_denied" => OAuthError::AccessDenied,
            other => OAuthError::Refused(short_code(other)),
        });
    }
    code.filter(|c| !c.is_empty())
        .ok_or(OAuthError::MissingCode)
}

/// A provider error code as shown: only the characters an OAuth error code has (RFC 6749 says
/// `%x20-21 / %x23-5B / %x5D-7E`; this keeps letters, digits and `_`), at most 40.
fn short_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect()
}

/// The form body that exchanges a code for tokens.
pub fn code_exchange_body(
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Zeroizing<String> {
    let mut body = Zeroizing::new(format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        form_encode(code),
        form_encode(redirect_uri),
        form_encode(client_id),
        form_encode(verifier),
    ));
    if let Some(s) = client_secret.filter(|s| !s.is_empty()) {
        body.push_str("&client_secret=");
        body.push_str(&form_encode(s));
    }
    body
}

/// The form body that refreshes an access token.
pub fn refresh_body(
    provider: OAuthProvider,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Zeroizing<String> {
    let mut body = Zeroizing::new(format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        form_encode(refresh_token),
        form_encode(client_id),
    ));
    if provider == OAuthProvider::Microsoft {
        // Microsoft wants the scopes again on a refresh.
        body.push_str("&scope=");
        body.push_str(&form_encode(&provider.scopes().join(" ")));
    }
    if let Some(s) = client_secret.filter(|s| !s.is_empty()) {
        body.push_str("&client_secret=");
        body.push_str(&form_encode(s));
    }
    body
}

/// Tokens from the token endpoint.
#[derive(Clone)]
pub struct Tokens {
    /// The access token (sent with XOAUTH2).
    pub access_token: Zeroizing<String>,
    /// The refresh token, when the provider sent one (a new one replaces the old: rotation).
    pub refresh_token: Option<Zeroizing<String>>,
    /// When the access token expires, Unix milliseconds.
    pub expires_at_ms: i64,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"***")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "***"))
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(serde::Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
}

/// Reads the token endpoint's answer: tokens on 200; `invalid_grant` is [`OAuthError::InvalidGrant`];
/// other 4xx errors are [`OAuthError::Refused`] with the error code; 5xx and 429 are transient.
pub fn parse_token_answer(status: u16, body: &[u8], now_ms: i64) -> Result<Tokens, OAuthError> {
    if status >= 500 || status == 429 {
        return Err(OAuthError::Transient);
    }
    let answer: TokenAnswer = serde_json::from_slice(body).map_err(|_| OAuthError::Malformed)?;
    // The strings are wiped when this function returns or when the tokens are dropped.
    let access = answer.access_token.map(Zeroizing::new);
    let refresh = answer.refresh_token.map(Zeroizing::new);
    if status != 200 {
        return Err(match answer.error.as_deref() {
            Some("invalid_grant") => OAuthError::InvalidGrant,
            Some(code) => OAuthError::Refused(short_code(code)),
            None => OAuthError::Malformed,
        });
    }
    let access_token = access
        .filter(|a| !a.is_empty())
        .ok_or(OAuthError::Malformed)?;
    // A missing lifetime is taken as one hour, the usual one.
    let expires_in = answer.expires_in.unwrap_or(3600).clamp(0, 365 * 24 * 3600);
    Ok(Tokens {
        access_token,
        refresh_token: refresh.filter(|r| !r.is_empty()),
        expires_at_ms: now_ms.saturating_add(expires_in.saturating_mul(1000)),
    })
}

/// The XOAUTH2 string (before base64) for IMAP `AUTHENTICATE` and SMTP `AUTH`:
/// `user=<user>^Aauth=Bearer <token>^A^A`.
pub fn xoauth2_string(user: &str, access_token: &str) -> Zeroizing<String> {
    Zeroizing::new(format!("user={user}\x01auth=Bearer {access_token}\x01\x01"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
