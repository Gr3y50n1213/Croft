// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recording stand-ins for the network, for tests of discovery (this crate's and its callers').
//!
//! [`FakeResolver`] and [`FakeHttp`] answer from tables the test fills in and keep a list of every
//! question they were asked. That list is how the tests prove which names and URLs discovery
//! contacts. Nothing here touches a socket.

use super::net::{
    FetchError, FetchLimits, HttpFetch, HttpResponse, HttpsUrl, NetFuture, ResolveError, Resolver,
    SrvRecord,
};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
struct ResolverState {
    srv: HashMap<String, Result<Vec<SrvRecord>, ResolveError>>,
    ips: HashMap<String, Result<Vec<IpAddr>, ResolveError>>,
    delays: HashMap<String, Duration>,
    srv_asked: Vec<String>,
    ip_asked: Vec<String>,
}

/// A [`Resolver`] that answers from a table. A name that was not set answers
/// [`ResolveError::NotFound`].
#[derive(Default)]
pub struct FakeResolver {
    state: Mutex<ResolverState>,
}

impl FakeResolver {
    /// An empty resolver: every name is not found.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the SRV records for a name such as `_imaps._tcp.example.org`.
    pub fn set_srv(&self, name: &str, records: Vec<SrvRecord>) {
        lock(&self.state).srv.insert(name.to_string(), Ok(records));
    }

    /// Makes the SRV lookup for a name fail with `error`.
    pub fn fail_srv(&self, name: &str, error: ResolveError) {
        lock(&self.state).srv.insert(name.to_string(), Err(error));
    }

    /// Sets the addresses of a host.
    pub fn set_ips(&self, host: &str, ips: Vec<IpAddr>) {
        lock(&self.state).ips.insert(host.to_string(), Ok(ips));
    }

    /// Makes every lookup of this name (SRV or address) wait this long first.
    pub fn set_delay(&self, name: &str, delay: Duration) {
        lock(&self.state).delays.insert(name.to_string(), delay);
    }

    /// Every SRV name asked so far, in order.
    pub fn srv_queries(&self) -> Vec<String> {
        lock(&self.state).srv_asked.clone()
    }

    /// Every host whose addresses were asked for so far, in order.
    pub fn ip_queries(&self) -> Vec<String> {
        lock(&self.state).ip_asked.clone()
    }
}

impl Resolver for FakeResolver {
    fn lookup_srv<'a>(
        &'a self,
        name: &'a str,
    ) -> NetFuture<'a, Result<Vec<SrvRecord>, ResolveError>> {
        Box::pin(async move {
            let (answer, delay) = {
                let mut s = lock(&self.state);
                s.srv_asked.push(name.to_string());
                (
                    s.srv
                        .get(name)
                        .cloned()
                        .unwrap_or(Err(ResolveError::NotFound)),
                    s.delays.get(name).copied(),
                )
            };
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            answer
        })
    }

    fn lookup_ip<'a>(&'a self, host: &'a str) -> NetFuture<'a, Result<Vec<IpAddr>, ResolveError>> {
        Box::pin(async move {
            let (answer, delay) = {
                let mut s = lock(&self.state);
                s.ip_asked.push(host.to_string());
                (
                    s.ips
                        .get(host)
                        .cloned()
                        .unwrap_or(Err(ResolveError::NotFound)),
                    s.delays.get(host).copied(),
                )
            };
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            answer
        })
    }
}

enum Route {
    Response(HttpResponse),
    Error(FetchError),
}

#[derive(Default)]
struct HttpState {
    routes: HashMap<String, Route>,
    delays: HashMap<String, Duration>,
    asked: Vec<String>,
}

/// An [`HttpFetch`] that answers from a table keyed by the full URL (`https://host/path`). A URL
/// that was not set answers [`FetchError::NameNotFound`], like a host that does not exist.
#[derive(Default)]
pub struct FakeHttp {
    state: Mutex<HttpState>,
}

impl FakeHttp {
    /// An empty server set: every host is unknown.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `200` with `body` at `url`.
    pub fn set_body(&self, url: &str, body: &[u8]) {
        self.set_response(
            url,
            HttpResponse {
                status: 200,
                location: None,
                body: body.to_vec(),
            },
        );
    }

    /// Answers with a redirect (`status` such as 301 or 307) to `location`.
    pub fn set_redirect(&self, url: &str, status: u16, location: &str) {
        self.set_response(
            url,
            HttpResponse {
                status,
                location: Some(location.to_string()),
                body: Vec::new(),
            },
        );
    }

    /// Answers with only a status code (for example 404 or 500).
    pub fn set_status(&self, url: &str, status: u16) {
        self.set_response(
            url,
            HttpResponse {
                status,
                location: None,
                body: Vec::new(),
            },
        );
    }

    /// Answers with exactly this response.
    pub fn set_response(&self, url: &str, response: HttpResponse) {
        lock(&self.state)
            .routes
            .insert(url.to_string(), Route::Response(response));
    }

    /// Makes the request fail with `error`.
    pub fn set_error(&self, url: &str, error: FetchError) {
        lock(&self.state)
            .routes
            .insert(url.to_string(), Route::Error(error));
    }

    /// Makes the request wait this long before answering.
    pub fn set_delay(&self, url: &str, delay: Duration) {
        lock(&self.state).delays.insert(url.to_string(), delay);
    }

    /// Every URL requested so far, in order, as text.
    pub fn requests(&self) -> Vec<String> {
        lock(&self.state).asked.clone()
    }
}

impl HttpFetch for FakeHttp {
    fn get<'a>(
        &'a self,
        url: &'a HttpsUrl,
        limits: FetchLimits,
    ) -> NetFuture<'a, Result<HttpResponse, FetchError>> {
        Box::pin(async move {
            let key = url.as_text();
            let (answer, delay) = {
                let mut s = lock(&self.state);
                s.asked.push(key.clone());
                let answer = match s.routes.get(&key) {
                    Some(Route::Response(r)) => Ok(r.clone()),
                    Some(Route::Error(e)) => Err(*e),
                    None => Err(FetchError::NameNotFound),
                };
                (answer, s.delays.get(&key).copied())
            };
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            match answer {
                Ok(r) if r.body.len() > limits.max_body => Err(FetchError::TooLarge),
                other => other,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_fakes_answer_from_their_tables_and_remember_the_questions() {
        let dns = FakeResolver::new();
        dns.set_srv(
            "_imaps._tcp.example.org",
            vec![SrvRecord {
                priority: 1,
                weight: 1,
                port: 993,
                target: "mail.example.org".into(),
            }],
        );
        assert_eq!(
            dns.lookup_srv("_imaps._tcp.example.org")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            dns.lookup_srv("_imap._tcp.example.org").await,
            Err(ResolveError::NotFound)
        );
        assert_eq!(
            dns.srv_queries(),
            ["_imaps._tcp.example.org", "_imap._tcp.example.org"]
        );

        let http = FakeHttp::new();
        http.set_body("https://example.org/a", b"hi");
        let limits = FetchLimits {
            timeout: Duration::from_secs(1),
            max_body: 1,
        };
        let u = HttpsUrl::new("example.org", "/a").unwrap();
        assert_eq!(http.get(&u, limits).await, Err(FetchError::TooLarge));
        let missing = HttpsUrl::new("example.org", "/b").unwrap();
        assert_eq!(
            http.get(&missing, limits).await,
            Err(FetchError::NameNotFound)
        );
        assert_eq!(
            http.requests(),
            ["https://example.org/a", "https://example.org/b"]
        );
    }
}
