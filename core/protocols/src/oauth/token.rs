// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Talking to a provider's token endpoint: exchanging the code, refreshing. Requests go only to
//! the provider's token host from the table in [`super`] over https with the usual certificate
//! rules (or, for tests, to a loopback address), never anywhere else, never through a redirect.

use std::time::Duration;

use zeroize::Zeroizing;

use super::{
    code_exchange_body, parse_token_answer, refresh_body, OAuthError, OAuthProvider, Tokens,
};
use crate::jmap::http::{Http, HttpRequest};
use crate::jmap::tls::Trust;
use crate::jmap::url::{is_loopback_name, Url};
use crate::types::BackendError;

/// Each wait for the endpoint.
const TIMEOUT: Duration = Duration::from_secs(15);
/// The whole request.
const DEADLINE: Duration = Duration::from_secs(30);
/// Largest answer read (a token answer is a few kilobytes).
const MAX_ANSWER: usize = 64 * 1024;

/// The person's own client: its id (not a secret) and, for a Google desktop-type client, the
/// secret Google gives such a client (kept in the keystore; never shipped by Croft).
#[derive(Clone)]
pub struct OAuthClient {
    /// The client id.
    pub client_id: String,
    /// The client secret, when the client type has one.
    pub client_secret: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &"***")
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Where tokens come from for one provider.
pub struct TokenEndpoint {
    provider: OAuthProvider,
    url: Url,
    http: Http,
}

impl TokenEndpoint {
    /// The provider's own token endpoint.
    pub fn for_provider(provider: OAuthProvider) -> Result<Self, BackendError> {
        let url = Url::parse(provider.token_url())
            .ok_or_else(|| BackendError::Permanent("bad token endpoint".into()))?;
        Ok(Self {
            provider,
            url,
            http: Http::new(Trust::Roots, TIMEOUT, DEADLINE),
        })
    }

    /// A token endpoint on this device (`http://127.0.0.1:<port>/...`), for tests. Any other
    /// address is refused, so this cannot be used to send a code or a token off the device.
    pub fn loopback(provider: OAuthProvider, url: &str) -> Option<Self> {
        let url = Url::parse(url)?;
        if url.secure || !is_loopback_name(&url.host) {
            return None;
        }
        Some(Self {
            provider,
            url,
            http: Http::new(Trust::Roots, Duration::from_secs(2), Duration::from_secs(4)),
        })
    }

    /// Exchanges an authorisation code for tokens.
    pub async fn exchange_code(
        &self,
        client: &OAuthClient,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
        now_ms: i64,
    ) -> Result<Tokens, OAuthError> {
        let body = code_exchange_body(
            &client.client_id,
            client.client_secret.as_deref().map(String::as_str),
            code,
            redirect_uri,
            verifier,
        );
        self.post(&body, now_ms).await
    }

    /// A new access token from the refresh token. A rotated refresh token is in the answer; the
    /// caller hands it to the keystore.
    pub async fn refresh(
        &self,
        client: &OAuthClient,
        refresh_token: &str,
        now_ms: i64,
    ) -> Result<Tokens, OAuthError> {
        let body = refresh_body(
            self.provider,
            &client.client_id,
            client.client_secret.as_deref().map(String::as_str),
            refresh_token,
        );
        self.post(&body, now_ms).await
    }

    async fn post(&self, body: &Zeroizing<String>, now_ms: i64) -> Result<Tokens, OAuthError> {
        let req = HttpRequest {
            method: "POST",
            url: &self.url,
            authorization: None,
            content_type: Some("application/x-www-form-urlencoded"),
            body: body.as_bytes(),
            max_response: MAX_ANSWER,
            extra_time: Duration::ZERO,
            error_body: true,
            status_only: false,
            extra_headers: &[],
        };
        let answer = self.http.send(&req).await.map_err(|e| match e.error {
            BackendError::Transient(_) => OAuthError::Transient,
            _ => OAuthError::Refused("connection".into()),
        })?;
        // The endpoint never redirects a token request; a redirect is not followed.
        if (300..400).contains(&answer.status) {
            return Err(OAuthError::Refused("redirect".into()));
        }
        // The answer holds the tokens: wiped once read.
        let body = Zeroizing::new(answer.body);
        parse_token_answer(answer.status, &body, now_ms)
    }
}

impl std::fmt::Debug for TokenEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenEndpoint")
            .field("provider", &self.provider)
            .field("host", &self.url.host)
            .finish()
    }
}
