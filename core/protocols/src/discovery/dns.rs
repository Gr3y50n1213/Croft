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
use std::net::IpAddr;

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

    /// Uses the given name servers (port 53, UDP with TCP fallback) instead of the system's.
    pub fn with_name_servers(servers: &[IpAddr]) -> Result<Self, ResolveError> {
        if servers.is_empty() {
            return Err(ResolveError::Failed);
        }
        let name_servers = servers
            .iter()
            .map(|ip| {
                NameServerConfig::new(
                    *ip,
                    true,
                    vec![ConnectionConfig::udp(), ConnectionConfig::tcp()],
                )
            })
            .collect();
        let config = ResolverConfig::from_parts(None, vec![], name_servers);
        let inner = Inner::builder_with_config(config, TokioRuntimeProvider::default())
            .build()
            .map_err(|_| ResolveError::Failed)?;
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
