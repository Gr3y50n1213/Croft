// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The real [`Resolver`]: `hickory-resolver` over plain DNS (UDP, with TCP fallback), using the
//! device's own name servers.
//!
//! Every name is sent fully qualified (a trailing dot is added), so the system's search list and
//! `ndots` rule never turn `_imaps._tcp.example.org` into a query for
//! `_imaps._tcp.example.org.corp.local`.
//!
//! Which name servers: on Linux, macOS and Windows the system configuration is read. On Android
//! `hickory-resolver` reads it through JNI (the `system-config` feature pulls in the Android
//! glue); if that is not possible on a given device, build the resolver with
//! [`HickoryResolver::with_name_servers`] and the servers the platform layer reports.

use super::net::{NetFuture, ResolveError, Resolver, SrvRecord};
use hickory_resolver::config::{ConnectionConfig, NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::NetError;
use hickory_resolver::proto::rr::RData;
use hickory_resolver::{Resolver as Inner, TokioResolver};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// DNS lookups through the operating system's name servers.
pub struct HickoryResolver {
    inner: TokioResolver,
}

impl HickoryResolver {
    /// Reads the name servers from the system configuration. Fails when that cannot be read.
    pub fn from_system() -> Result<Self, ResolveError> {
        let inner = Inner::builder_tokio()
            .and_then(|b| b.build())
            .map_err(|_| ResolveError::Failed)?;
        Ok(Self { inner })
    }

    /// Uses the given name servers (UDP, with TCP fallback) instead of the system's. Use port 53
    /// unless the platform says otherwise. Each query waits 3 seconds for an answer and is tried
    /// twice, which stays inside discovery's 10 second request limit.
    pub fn with_name_servers(servers: &[SocketAddr]) -> Result<Self, ResolveError> {
        Self::with_name_servers_and_timing(servers, Duration::from_secs(3), 2)
    }

    fn with_name_servers_and_timing(
        servers: &[SocketAddr],
        timeout: Duration,
        attempts: usize,
    ) -> Result<Self, ResolveError> {
        if servers.is_empty() {
            return Err(ResolveError::Failed);
        }
        let name_servers = servers
            .iter()
            .map(|addr| {
                let mut udp = ConnectionConfig::udp();
                udp.port = addr.port();
                let mut tcp = ConnectionConfig::tcp();
                tcp.port = addr.port();
                NameServerConfig::new(addr.ip(), true, vec![udp, tcp])
            })
            .collect();
        let config = ResolverConfig::from_parts(None, vec![], name_servers);
        let mut builder = Inner::builder_with_config(config, TokioRuntimeProvider::default());
        builder.options_mut().timeout = timeout;
        builder.options_mut().attempts = attempts;
        let inner = builder.build().map_err(|_| ResolveError::Failed)?;
        Ok(Self { inner })
    }
}

impl std::fmt::Debug for HickoryResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HickoryResolver")
    }
}

fn map_error(e: &NetError) -> ResolveError {
    if e.is_no_records_found() {
        ResolveError::NotFound
    } else if matches!(e, NetError::Timeout) {
        ResolveError::TimedOut
    } else {
        ResolveError::Failed
    }
}

fn fully_qualified(name: &str) -> String {
    if name.ends_with('.') {
        name.to_string()
    } else {
        format!("{name}.")
    }
}

impl Resolver for HickoryResolver {
    fn lookup_srv<'a>(
        &'a self,
        name: &'a str,
    ) -> NetFuture<'a, Result<Vec<SrvRecord>, ResolveError>> {
        Box::pin(async move {
            let lookup = self
                .inner
                .srv_lookup(fully_qualified(name))
                .await
                .map_err(|e| map_error(&e))?;
            let records = lookup
                .answers()
                .iter()
                .filter_map(|record| match &record.data {
                    RData::SRV(srv) => Some(SrvRecord {
                        priority: srv.priority,
                        weight: srv.weight,
                        port: srv.port,
                        target: srv
                            .target
                            .to_ascii()
                            .trim_end_matches('.')
                            .to_ascii_lowercase(),
                    }),
                    _ => None,
                })
                .collect();
            Ok(records)
        })
    }

    fn lookup_ip<'a>(&'a self, host: &'a str) -> NetFuture<'a, Result<Vec<IpAddr>, ResolveError>> {
        Box::pin(async move {
            let lookup = self
                .inner
                .lookup_ip(fully_qualified(host))
                .await
                .map_err(|e| map_error(&e))?;
            Ok(lookup.iter().collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::proto::op::{Message, ResponseCode};
    use hickory_resolver::proto::rr::rdata::{A, SRV};
    use hickory_resolver::proto::rr::{Name, RData, Record, RecordType};
    use std::sync::{Arc, Mutex};
    use tokio::net::UdpSocket;

    /// A tiny DNS server on a local UDP port: two SRV records (one of them the root name) for
    /// `_imaps._tcp.example.test`, an address for `host.example.test`, and "no such name" for
    /// everything else. Returns its address and the list of names it was asked about.
    async fn dns_server() -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            while let Ok((n, from)) = socket.recv_from(&mut buf).await {
                let Ok(query) = Message::from_vec(&buf[..n]) else {
                    continue;
                };
                let mut reply = query.clone().into_response();
                let Some(q) = query.queries.first() else {
                    continue;
                };
                let name = q.name().to_ascii();
                log.lock().unwrap().push(name.clone());
                match (name.as_str(), q.query_type()) {
                    ("_imaps._tcp.example.test.", RecordType::SRV) => {
                        let n = |s: &str| Name::from_ascii(s).unwrap();
                        reply.add_answer(Record::from_rdata(
                            q.name().clone(),
                            60,
                            RData::SRV(SRV::new(10, 5, 993, n("Mail.Example.Test."))),
                        ));
                        reply.add_answer(Record::from_rdata(
                            q.name().clone(),
                            60,
                            RData::SRV(SRV::new(20, 0, 0, Name::root())),
                        ));
                    }
                    ("host.example.test.", RecordType::A) => {
                        reply.add_answer(Record::from_rdata(
                            q.name().clone(),
                            60,
                            RData::A(A::new(192, 0, 2, 7)),
                        ));
                    }
                    ("host.example.test.", _) => {}
                    _ => reply.metadata.response_code = ResponseCode::NXDomain,
                }
                let _ = socket.send_to(&reply.to_vec().unwrap(), from).await;
            }
        });
        (addr, asked)
    }

    #[tokio::test]
    async fn the_real_resolver_reads_srv_and_address_answers() {
        let (addr, asked) = dns_server().await;
        let resolver = HickoryResolver::with_name_servers(&[addr]).unwrap();

        let records = resolver
            .lookup_srv("_imaps._tcp.example.test")
            .await
            .unwrap();
        assert_eq!(records.len(), 2);
        let mail = records.iter().find(|r| r.port == 993).unwrap();
        assert_eq!(
            (mail.priority, mail.weight, mail.target.as_str()),
            (10, 5, "mail.example.test")
        );
        // The root name comes through as an empty target: "not offered".
        assert!(records.iter().any(|r| r.target.is_empty()));

        assert_eq!(
            resolver.lookup_ip("host.example.test").await.unwrap(),
            vec![IpAddr::from([192, 0, 2, 7])]
        );
        assert_eq!(
            resolver.lookup_srv("_imap._tcp.example.test").await,
            Err(ResolveError::NotFound)
        );
        assert_eq!(
            resolver.lookup_ip("nothing.example.test").await,
            Err(ResolveError::NotFound)
        );

        // Every query went out fully qualified and exactly as asked: no search suffix added.
        let asked = asked.lock().unwrap().clone();
        assert!(
            asked.iter().all(|n| n.ends_with("example.test.")),
            "{asked:?}"
        );
        assert!(asked.contains(&"_imaps._tcp.example.test.".to_string()));
    }

    #[tokio::test]
    async fn an_unreachable_name_server_is_a_failure_not_a_missing_name() {
        // A port nobody listens on: UDP gets no answer, TCP is refused.
        let port = {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            s.local_addr().unwrap().port()
        };
        let resolver = HickoryResolver::with_name_servers_and_timing(
            &[SocketAddr::from(([127, 0, 0, 1], port))],
            Duration::from_millis(300),
            1,
        )
        .unwrap();
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            resolver.lookup_srv("_imaps._tcp.example.test"),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_ne!(err, ResolveError::NotFound);
        assert!(HickoryResolver::with_name_servers(&[]).is_err());
    }
}
