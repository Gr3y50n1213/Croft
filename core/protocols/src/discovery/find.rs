// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Running the probes together and choosing the winner. See the module documentation of
//! [`discovery`](super) for the rules; the code here is those rules and nothing else.

use super::address::EmailAddress;
use super::autoconfig;
use super::dns::HickoryResolver;
use super::error::{DiscoveryError, Probe, ProbeFailure, ProbeState, ProbeUpdate};
use super::http::HttpsFetcher;
use super::net::{
    FetchError, FetchLimits, HttpFetch, HttpResponse, HttpsUrl, ResolveError, Resolver, SrvRecord,
};
use super::srv::{self, SrvAnswers};
use super::{provider, AuthHint, DiscoveredSettings, ImapServer, SmtpServer, Source};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio::time::{timeout, timeout_at, Instant};

/// How long and how much discovery may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest one DNS query or one HTTPS request may take. Default 10 seconds.
    pub request_timeout: Duration,
    /// Longest the whole search may take. Default 20 seconds. When it runs out, probes still
    /// running are reported as timed out and a later probe that already finished can still win.
    pub overall_timeout: Duration,
    /// Largest response body read. Default 64 KiB.
    pub max_response: usize,
    /// Most redirects followed for one file. Default 3.
    pub max_redirects: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(10),
            overall_timeout: Duration::from_secs(20),
            max_response: autoconfig::MAX_BYTES,
            max_redirects: 3,
        }
    }
}

/// Finds server settings for an email address. Cheap to build; holds no state between calls.
pub struct Discoverer {
    resolver: Arc<dyn Resolver>,
    http: Arc<dyn HttpFetch>,
    limits: Limits,
    progress: Option<UnboundedSender<ProbeUpdate>>,
}

impl std::fmt::Debug for Discoverer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discoverer")
            .field("limits", &self.limits)
            .finish()
    }
}

/// What a probe found, before it is turned into [`DiscoveredSettings`].
struct Found {
    imap: Option<ImapServer>,
    smtp: Option<SmtpServer>,
    jmap: Option<String>,
    auth: AuthHint,
    source: Source,
}

type ProbeResult = Result<Found, ProbeFailure>;

/// Everything a probe task needs, shared by the four tasks.
struct Context {
    resolver: Arc<dyn Resolver>,
    http: Arc<dyn HttpFetch>,
    limits: Limits,
    address: EmailAddress,
    progress: Option<UnboundedSender<ProbeUpdate>>,
}

impl Context {
    fn emit(&self, probe: Probe, state: ProbeState) {
        if let Some(tx) = &self.progress {
            // A closed receiver means nobody is watching; discovery carries on.
            let _ = tx.send(ProbeUpdate { probe, state });
        }
    }
}

/// A running probe, stopped if discovery ends or is dropped before it does.
struct Task(JoinHandle<ProbeResult>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Discoverer {
    /// Discovery over the given network doors.
    pub fn new(resolver: Arc<dyn Resolver>, http: Arc<dyn HttpFetch>) -> Self {
        Self {
            resolver,
            http,
            limits: Limits::default(),
            progress: None,
        }
    }

    /// Discovery over the real network: the system's name servers and rustls. Fails with
    /// [`DiscoveryError::ResolverUnavailable`] when the system DNS settings cannot be read.
    pub fn with_system_network() -> Result<Self, DiscoveryError> {
        let resolver: Arc<dyn Resolver> = Arc::new(
            HickoryResolver::from_system().map_err(|_| DiscoveryError::ResolverUnavailable)?,
        );
        let http: Arc<dyn HttpFetch> = Arc::new(HttpsFetcher::new(resolver.clone()));
        Ok(Self::new(resolver, http))
    }

    /// Changes the limits.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sends a [`ProbeUpdate`] whenever a probe changes state, for the S03 progress list. The
    /// final state of every probe is sent before [`discover`](Self::discover) returns.
    pub fn with_progress(mut self, sender: UnboundedSender<ProbeUpdate>) -> Self {
        self.progress = Some(sender);
        self
    }

    /// Finds the settings for `address` (no password involved, nothing secret sent). See the
    /// module documentation for the order of preference and exactly which hosts are contacted.
    ///
    /// Must be called inside a Tokio runtime (the probes run as tasks). Dropping the returned
    /// future cancels every probe that is still running.
    pub async fn discover(&self, address: &str) -> Result<DiscoveredSettings, DiscoveryError> {
        let address = EmailAddress::parse(address)?;
        let emit = |probe, state| {
            if let Some(tx) = &self.progress {
                let _ = tx.send(ProbeUpdate { probe, state });
            }
        };

        // 1. The provider table needs no network, so nothing else is asked for these domains.
        if let Some(entry) = provider::lookup(address.domain()) {
            let full = address.full();
            emit(Probe::ProviderTable, ProbeState::Found);
            for p in &Probe::ALL[1..] {
                emit(*p, ProbeState::Skipped);
            }
            return Ok(DiscoveredSettings {
                imap: Some(entry.imap(full.clone())),
                smtp: Some(entry.smtp(full)),
                jmap: entry.jmap.map(str::to_string),
                source: Source::ProviderTable,
                provider: Some(entry.provider),
                auth_hint: entry.auth_hint,
            });
        }
        emit(Probe::ProviderTable, ProbeState::Skipped);

        // 2. The four network probes start together.
        let ctx = Arc::new(Context {
            resolver: self.resolver.clone(),
            http: self.http.clone(),
            limits: self.limits,
            address,
            progress: self.progress.clone(),
        });
        let order = [
            Probe::AutoconfigSubdomain,
            Probe::AutoconfigWellKnown,
            Probe::Srv,
            Probe::JmapWellKnown,
        ];
        for p in order {
            emit(p, ProbeState::Waiting);
        }
        let mut tasks: Vec<(Probe, Task)> = order
            .iter()
            .map(|&probe| {
                let ctx = ctx.clone();
                let handle = tokio::spawn(async move {
                    ctx.emit(probe, ProbeState::Running);
                    let result = run_probe(&ctx, probe).await;
                    ctx.emit(
                        probe,
                        match &result {
                            Ok(_) => ProbeState::Found,
                            Err(why) => ProbeState::Failed(*why),
                        },
                    );
                    result
                });
                (probe, Task(handle))
            })
            .collect();

        // 3. Take them in order of preference. A later probe's answer is held until every
        //    earlier probe has failed; the first earlier probe to succeed wins.
        let deadline = Instant::now() + self.limits.overall_timeout;
        let mut reasons: Vec<(Probe, ProbeFailure)> = Vec::new();
        let mut winner: Option<(usize, Found)> = None;
        for (i, (probe, task)) in tasks.iter_mut().enumerate() {
            let outcome = match timeout_at(deadline, &mut task.0).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(ProbeFailure::Internal),
                Err(_) => {
                    // Out of time: this probe is cancelled when `tasks` is dropped below.
                    emit(*probe, ProbeState::Failed(ProbeFailure::TimedOut));
                    Err(ProbeFailure::TimedOut)
                }
            };
            match outcome {
                Ok(found) => {
                    winner = Some((i, found));
                    break;
                }
                Err(why) => reasons.push((*probe, why)),
            }
        }

        match winner {
            Some((i, found)) => {
                for (probe, _) in &tasks[i + 1..] {
                    emit(*probe, ProbeState::Skipped);
                }
                Ok(DiscoveredSettings {
                    imap: found.imap,
                    smtp: found.smtp,
                    jmap: found.jmap,
                    source: found.source,
                    provider: None,
                    auth_hint: found.auth,
                })
            }
            None => {
                if reasons.iter().all(|(_, why)| why.is_connectivity()) {
                    Err(DiscoveryError::Offline { reasons })
                } else {
                    Err(DiscoveryError::NothingFound { reasons })
                }
            }
        }
    }
}

async fn run_probe(ctx: &Context, probe: Probe) -> ProbeResult {
    let domain = ctx.address.domain();
    match probe {
        Probe::AutoconfigSubdomain => {
            autoconfig_probe(
                ctx,
                &format!("autoconfig.{domain}"),
                "/mail/config-v1.1.xml",
            )
            .await
        }
        Probe::AutoconfigWellKnown => {
            autoconfig_probe(ctx, domain, "/.well-known/autoconfig/mail/config-v1.1.xml").await
        }
        Probe::Srv => srv_probe(ctx).await,
        Probe::JmapWellKnown => jmap_probe(ctx).await,
        // The provider table is answered before any probe runs.
        Probe::ProviderTable => Err(ProbeFailure::Internal),
    }
}

fn map_fetch_error(e: FetchError) -> ProbeFailure {
    match e {
        FetchError::NameNotFound => ProbeFailure::NotPublished,
        FetchError::Dns | FetchError::Connect => ProbeFailure::Unreachable,
        FetchError::Tls => ProbeFailure::CertificateProblem,
        FetchError::TimedOut => ProbeFailure::TimedOut,
        FetchError::TooLarge => ProbeFailure::TooLarge,
        FetchError::Protocol => ProbeFailure::Unusable,
    }
}

/// One request, under the per-request timeout.
async fn request(ctx: &Context, url: &HttpsUrl) -> Result<HttpResponse, ProbeFailure> {
    let limits = FetchLimits {
        timeout: ctx.limits.request_timeout,
        max_body: ctx.limits.max_response,
    };
    match timeout(ctx.limits.request_timeout, ctx.http.get(url, limits)).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(map_fetch_error(e)),
        Err(_) => Err(ProbeFailure::TimedOut),
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Turns a `Location` header into a URL: absolute `https://`, network-path (`//host/path`) or
/// absolute-path (`/path`, on the same host). Anything else (including `http://`) is `None`.
fn resolve_location(base: &HttpsUrl, location: &str, any_port: bool) -> Option<HttpsUrl> {
    let parse = |text: &str| {
        if any_port {
            HttpsUrl::parse_allowing_port(text)
        } else {
            HttpsUrl::parse(text)
        }
    };
    if location.starts_with("https://") {
        parse(location)
    } else if location.starts_with("//") {
        parse(&format!("https:{location}"))
    } else if location.starts_with('/') {
        parse(&format!("https://{}{location}", base.host()))
    } else {
        None
    }
}

/// Fetches a file, following up to `max_redirects` redirects, each to `https` on the person's
/// own domain or a subdomain of it, and returns the `200` response.
async fn fetch_following(ctx: &Context, start: HttpsUrl) -> Result<HttpResponse, ProbeFailure> {
    let domain = ctx.address.domain();
    let mut url = start;
    let mut hops = 0u8;
    loop {
        let resp = request(ctx, &url).await?;
        match resp.status {
            200 => return Ok(resp),
            s if is_redirect(s) => {
                hops += 1;
                if hops > ctx.limits.max_redirects {
                    return Err(ProbeFailure::BadRedirect);
                }
                let next = resp
                    .location
                    .as_deref()
                    .and_then(|l| resolve_location(&url, l, false))
                    .filter(|u| u.is_within(domain))
                    .ok_or(ProbeFailure::BadRedirect)?;
                url = next;
            }
            404 | 410 => return Err(ProbeFailure::NotPublished),
            _ => return Err(ProbeFailure::Unusable),
        }
    }
}

async fn autoconfig_probe(ctx: &Context, host: &str, path: &str) -> ProbeResult {
    let start = HttpsUrl::new(host, path).ok_or(ProbeFailure::Internal)?;
    let resp = fetch_following(ctx, start).await?;
    let parsed = autoconfig::parse(&resp.body, &ctx.address)?;
    Ok(Found {
        imap: Some(parsed.imap),
        smtp: parsed.smtp,
        jmap: None,
        auth: parsed.auth,
        source: Source::Autoconfig,
    })
}

/// One SRV lookup under the per-request timeout. "No such record" is an empty list.
async fn srv_lookup(ctx: &Context, service: &str) -> Result<Vec<SrvRecord>, ProbeFailure> {
    let name = format!("{service}.{}", ctx.address.domain());
    match timeout(ctx.limits.request_timeout, ctx.resolver.lookup_srv(&name)).await {
        Ok(Ok(records)) => Ok(records),
        Ok(Err(ResolveError::NotFound)) => Ok(Vec::new()),
        Ok(Err(ResolveError::TimedOut)) | Err(_) => Err(ProbeFailure::TimedOut),
        Ok(Err(ResolveError::Failed)) => Err(ProbeFailure::Unreachable),
    }
}

async fn srv_probe(ctx: &Context) -> ProbeResult {
    let (imaps, imap, submissions, submission, jmap) = futures_util::future::join5(
        srv_lookup(ctx, "_imaps._tcp"),
        srv_lookup(ctx, "_imap._tcp"),
        srv_lookup(ctx, "_submissions._tcp"),
        srv_lookup(ctx, "_submission._tcp"),
        srv_lookup(ctx, "_jmap._tcp"),
    )
    .await;
    let mut failure = None;
    let mut take = |r: Result<Vec<SrvRecord>, ProbeFailure>| match r {
        Ok(v) => v,
        Err(why) => {
            failure.get_or_insert(why);
            Vec::new()
        }
    };
    let answers = SrvAnswers {
        imaps: take(imaps),
        imap: take(imap),
        submissions: take(submissions),
        submission: take(submission),
        jmap: take(jmap),
    };
    match srv::settings(&answers, &ctx.address.full(), &mut srv::random_below) {
        Ok(s) => Ok(Found {
            imap: s.imap,
            smtp: s.smtp,
            jmap: s.jmap,
            auth: AuthHint::Password,
            source: Source::Srv,
        }),
        // Nothing usable: if a lookup failed, that is the more useful reason.
        Err(why) => Err(failure.unwrap_or(why)),
    }
}

async fn jmap_probe(ctx: &Context) -> ProbeResult {
    let url =
        HttpsUrl::new(ctx.address.domain(), "/.well-known/jmap").ok_or(ProbeFailure::Internal)?;
    // No redirect is followed: the redirect target is the answer (RFC 8620 section 2.2), and it
    // may well be on another host, such as a provider's API host. It is recorded, not contacted.
    let resp = request(ctx, &url).await?;
    let jmap = match resp.status {
        s if is_redirect(s) => resp
            .location
            .as_deref()
            .and_then(|l| resolve_location(&url, l, true))
            .map(|u| u.as_text())
            .ok_or(ProbeFailure::BadRedirect)?,
        200 => {
            let looks_like_a_session = std::str::from_utf8(&resp.body)
                .is_ok_and(|t| t.contains("urn:ietf:params:jmap:core"));
            if !looks_like_a_session {
                return Err(ProbeFailure::Unusable);
            }
            url.as_text()
        }
        404 | 410 => return Err(ProbeFailure::NotPublished),
        _ => return Err(ProbeFailure::Unusable),
    };
    Ok(Found {
        imap: None,
        smtp: None,
        jmap: Some(jmap),
        auth: AuthHint::Password,
        source: Source::JmapWellKnown,
    })
}
