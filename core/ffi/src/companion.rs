// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The app's connection to a Croft Companion (T34b), over `croft_protocols::companion`.
//!
//! **Connecting.** [`Core::companion_probe`] checks an address (S06). [`Core::companion_begin`]
//! starts a sign-in and gives the Companion page to open in a Custom Tab; the app's redirect
//! activity hands the address it receives to [`Core::companion_finish`], which redeems the code
//! with the sign-in's PKCE verifier (kept here, never given to the app) and returns the
//! connection's **secret**: an opaque blob the app keeps in the keystore, like a password.
//!
//! **Using it.** [`Core::companion_link`] makes a [`CompanionLink`] from the secret. The link
//! renews its access token as needed; when the Companion replaces the refresh token, the new
//! secret goes to the app's [`CompanionSecretSaver`] (the old one stops working). A
//! [`CompanionFailure::SignedOut`] means the Companion ended this device's session: forget the
//! secret and offer to connect again.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use croft_protocols::companion::{self, CompanionError, Session, SignIn};
use tokio::runtime::Handle;

use crate::core::Core;

/// A sign-in that was begun and not finished is forgotten after this.
const FLOW_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// At most this many sign-ins wait at once (an older one is dropped).
const MAX_FLOWS: usize = 4;

/// Why a Companion call did not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum CompanionFailure {
    /// Not an address a Companion can have (`https://host[:port]`).
    BadAddress,
    /// Something answers there, but not a Croft Companion.
    NotACompanion,
    /// No answer: offline, a wrong address, or a certificate this device does not trust.
    Unreachable,
    /// The sign-in did not work or was cancelled (or took longer than ten minutes): try again.
    SignInFailed,
    /// The Companion ended this device's session: forget the secret, connect again.
    SignedOut,
    /// Too many requests: try again in a while.
    RateLimited,
    /// The Companion refused or failed.
    Server,
    /// Something inside the core failed.
    Internal,
}

impl std::fmt::Display for CompanionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CompanionFailure {}

impl From<CompanionError> for CompanionFailure {
    fn from(e: CompanionError) -> Self {
        match e {
            CompanionError::BadAddress => Self::BadAddress,
            CompanionError::NotACompanion => Self::NotACompanion,
            CompanionError::Unreachable => Self::Unreachable,
            CompanionError::SignInFailed => Self::SignInFailed,
            CompanionError::SignedOut => Self::SignedOut,
            CompanionError::RateLimited => Self::RateLimited,
            CompanionError::Server => Self::Server,
        }
    }
}

/// What a Companion says about itself (S06).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CompanionInfo {
    /// Its address, normalized (what to keep and show).
    pub url: String,
    /// Its version.
    pub version: String,
    /// The name of its single sign-on, when it has one.
    pub sso_name: Option<String>,
}

/// A sign-in begun: open `browser_url` in a Custom Tab, keep `flow_id` for the finish.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CompanionSignInStart {
    /// Names this sign-in for [`Core::companion_finish`].
    pub flow_id: String,
    /// The Companion's sign-in page.
    pub browser_url: String,
}

/// A connected Companion: keep `secret` in the keystore.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct CompanionConnected {
    /// The Companion's address.
    pub url: String,
    /// Who signed in.
    pub user_name: String,
    /// The connection's secret (opaque; never shown, logged or sent anywhere but back here).
    pub secret: Vec<u8>,
}

impl std::fmt::Debug for CompanionConnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanionConnected")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// One of the person's devices signed in to the Companion (S58).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CompanionDevice {
    /// The Companion's id for it.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// When it signed in, Unix milliseconds.
    pub created_ms: i64,
    /// When it was last seen.
    pub last_seen_ms: i64,
    /// This app.
    pub current: bool,
}

/// Implemented by the app: keeps the connection's new secret in place of the old one (the
/// keystore). Called from Rust threads.
#[uniffi::export(with_foreign)]
pub trait CompanionSecretSaver: Send + Sync {
    /// The Companion replaced the refresh token: `secret` is the connection's secret from now on.
    fn save(&self, secret: Vec<u8>);
}

/// The sign-ins in progress (their PKCE verifiers stay here).
static FLOWS: LazyLock<Mutex<HashMap<String, (Instant, SignIn)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The address as a Companion's (`https://host[:port]`), from what the person typed.
#[uniffi::export]
pub fn companion_address(typed: String) -> Result<String, CompanionFailure> {
    Ok(companion::normalize(&typed)?)
}

#[uniffi::export]
impl Core {
    /// Checks that a Croft Companion answers at `url` (S06): its address, version and single
    /// sign-on.
    pub async fn companion_probe(&self, url: String) -> Result<CompanionInfo, CompanionFailure> {
        let task = self.spawn(async move { companion::probe(&url).await }, true);
        let info = task.await.map_err(|_| CompanionFailure::Internal)??;
        Ok(CompanionInfo {
            url: info.url,
            version: info.version,
            sso_name: info.sso_name,
        })
    }

    /// Begins a sign-in to the Companion at `url` for the app whose redirect is `redirect`
    /// (`<applicationId>:/companion`).
    pub fn companion_begin(
        &self,
        url: String,
        redirect: String,
    ) -> Result<CompanionSignInStart, CompanionFailure> {
        let sign_in = SignIn::begin(&url, &redirect)?;
        let browser_url = sign_in.browser_url();
        let flow_id =
            croft_protocols::oauth::random_token().map_err(|_| CompanionFailure::Internal)?;
        let mut flows = FLOWS.lock().unwrap_or_else(PoisonError::into_inner);
        flows.retain(|_, (at, _)| at.elapsed() < FLOW_LIFETIME);
        while flows.len() >= MAX_FLOWS {
            let oldest = flows
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => flows.remove(&k),
                None => break,
            };
        }
        flows.insert(flow_id.clone(), (Instant::now(), sign_in));
        Ok(CompanionSignInStart {
            flow_id,
            browser_url,
        })
    }

    /// Finishes a sign-in with the address the browser came back to (`<redirect>?code=...`);
    /// `device` names this device in the person's list. A flow is used once: a second finish, or
    /// one after ten minutes, is [`CompanionFailure::SignInFailed`]. A callback whose code the
    /// Companion refuses (or that has none) leaves the flow waiting for the real one.
    pub async fn companion_finish(
        &self,
        flow_id: String,
        callback: String,
        device: String,
    ) -> Result<CompanionConnected, CompanionFailure> {
        let sign_in = {
            let mut flows = FLOWS.lock().unwrap_or_else(PoisonError::into_inner);
            flows.retain(|_, (at, _)| at.elapsed() < FLOW_LIFETIME);
            flows.remove(&flow_id)
        }
        .ok_or(CompanionFailure::SignInFailed)?;
        let task = self.spawn(
            async move {
                let (started, sign_in) = sign_in;
                let session = match sign_in.finish(&callback, &device).await {
                    Ok(s) => s,
                    Err(CompanionError::SignInFailed) => {
                        // Not this flow's code (a stray or forged redirect): the real one may
                        // still come, so the flow stays until it is used or expires.
                        FLOWS
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(flow_id, (started, sign_in));
                        return Err(CompanionError::SignInFailed);
                    }
                    Err(e) => return Err(e),
                };
                let user_name = match session.user_name().await {
                    Ok(n) => n,
                    Err(e) => {
                        // The session is of no use without its secret kept: end it rather than
                        // leave a device the person cannot use in their list.
                        let _ = session.sign_out().await;
                        return Err(e);
                    }
                };
                let secret = session.secret().await.to_vec();
                Ok::<_, CompanionError>(CompanionConnected {
                    url: session.url().to_owned(),
                    user_name,
                    secret,
                })
            },
            true,
        );
        Ok(task.await.map_err(|_| CompanionFailure::Internal)??)
    }

    /// The connection made from a kept secret; new secrets go to `saver`.
    /// [`CompanionFailure::SignedOut`] when the secret is not one (forget it).
    pub fn companion_link(
        &self,
        secret: Vec<u8>,
        saver: Arc<dyn CompanionSecretSaver>,
    ) -> Result<Arc<CompanionLink>, CompanionFailure> {
        let session = Session::from_secret(&secret).ok_or(CompanionFailure::SignedOut)?;
        session.set_saver(Box::new(move |s| saver.save(s.to_vec())));
        Ok(Arc::new(CompanionLink {
            session: Arc::new(session),
            rt: self.runtime_handle(),
        }))
    }
}

/// The app's connection to its Companion.
#[derive(uniffi::Object)]
pub struct CompanionLink {
    session: Arc<Session>,
    rt: Handle,
}

impl std::fmt::Debug for CompanionLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CompanionLink")
    }
}

impl CompanionLink {
    async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(
                Arc<Session>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<T, CompanionError>> + Send>,
            > + Send
            + 'static,
    ) -> Result<T, CompanionFailure> {
        let session = self.session.clone();
        let handle = self.rt.spawn(async move { f(session).await });
        let abort = handle.abort_handle();
        // The call is cancelled when the app stops waiting for it.
        struct AbortOnDrop(tokio::task::AbortHandle);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _guard = AbortOnDrop(abort);
        Ok(handle.await.map_err(|_| CompanionFailure::Internal)??)
    }
}

#[uniffi::export]
impl CompanionLink {
    /// The Companion's address.
    pub fn url(&self) -> String {
        self.session.url().to_owned()
    }

    /// Who is signed in.
    pub async fn user_name(&self) -> Result<String, CompanionFailure> {
        self.run(|s| Box::pin(async move { s.user_name().await }))
            .await
    }

    /// The person's signed-in devices (S58).
    pub async fn devices(&self) -> Result<Vec<CompanionDevice>, CompanionFailure> {
        self.run(|s| {
            Box::pin(async move {
                Ok(s.devices()
                    .await?
                    .into_iter()
                    .map(|d| CompanionDevice {
                        id: d.id,
                        name: d.name,
                        created_ms: d.created_ms,
                        last_seen_ms: d.last_seen_ms,
                        current: d.current,
                    })
                    .collect())
            })
        })
        .await
    }

    /// Signs another device out.
    pub async fn sign_out_device(&self, id: i64) -> Result<(), CompanionFailure> {
        self.run(move |s| Box::pin(async move { s.sign_out_device(id).await }))
            .await
    }

    /// Disconnects this device: the Companion ends its session (and its push subscriptions).
    /// Forget the secret whatever the answer.
    pub async fn sign_out(&self) -> Result<(), CompanionFailure> {
        self.run(|s| Box::pin(async move { s.sign_out().await }))
            .await
    }

    /// The Companion's push key (VAPID, base64url).
    pub async fn push_key(&self) -> Result<String, CompanionFailure> {
        self.run(|s| Box::pin(async move { s.push_key().await }))
            .await
    }

    /// Gives a push subscription to the Companion for this device; its id.
    pub async fn add_push(
        &self,
        endpoint: String,
        p256dh: String,
        auth: String,
    ) -> Result<i64, CompanionFailure> {
        self.run(move |s| Box::pin(async move { s.add_push(&endpoint, &p256dh, &auth).await }))
            .await
    }

    /// Removes a push subscription.
    pub async fn remove_push(&self, id: i64) -> Result<(), CompanionFailure> {
        self.run(move |s| Box::pin(async move { s.remove_push(id).await }))
            .await
    }
}
