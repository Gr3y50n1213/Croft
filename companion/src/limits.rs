// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Rate limits per client address (T24), in memory, for the endpoints that work without a token
//! (enrolment, sign-in, token refresh). Two rules: at most [`REQUESTS_PER_MINUTE`] requests a
//! minute, and after [`FAILURES_PER_WINDOW`] failures in [`FAILURE_WINDOW_MS`] the address is
//! refused until the oldest failure leaves the window. The per-user lockout is in the store.
//! IPv6 addresses count by their /64, since one host usually has a whole /64.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

/// Requests a minute from one address.
pub const REQUESTS_PER_MINUTE: u32 = 30;
/// Failures in the window from one address before it is refused.
pub const FAILURES_PER_WINDOW: usize = 20;
/// The failure window.
pub const FAILURE_WINDOW_MS: i64 = 15 * 60 * 1000;
/// Addresses remembered before old ones are forgotten.
const MAX_ADDRESSES: usize = 100_000;

/// The key an address counts under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Who {
    V4(u32),
    V6(u64),
}

fn who(ip: IpAddr) -> Who {
    match ip {
        IpAddr::V4(a) => Who::V4(u32::from(a)),
        IpAddr::V6(a) => match a.to_ipv4_mapped() {
            Some(v4) => Who::V4(u32::from(v4)),
            None => Who::V6((u128::from(a) >> 64) as u64),
        },
    }
}

#[derive(Debug, Default)]
struct Entry {
    minute: i64,
    requests: u32,
    failures: VecDeque<i64>,
}

/// The limits' state.
#[derive(Debug, Default)]
pub struct Limits {
    map: HashMap<Who, Entry>,
}

/// Why a request was refused, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    /// Seconds until a retry can work.
    pub retry_after_s: i64,
}

impl Limits {
    /// Counts a request from `ip`; refuses it when the address is over either limit.
    pub fn request(&mut self, ip: IpAddr, now: i64) -> Result<(), Refused> {
        self.forget_old(now);
        let e = self.map.entry(who(ip)).or_default();
        while e
            .failures
            .front()
            .is_some_and(|t| *t <= now - FAILURE_WINDOW_MS)
        {
            e.failures.pop_front();
        }
        if e.failures.len() >= FAILURES_PER_WINDOW {
            let oldest = e.failures.front().copied().unwrap_or(now);
            return Err(Refused {
                retry_after_s: ((oldest + FAILURE_WINDOW_MS - now) / 1000).max(1),
            });
        }
        let minute = now.div_euclid(60_000);
        if e.minute != minute {
            e.minute = minute;
            e.requests = 0;
        }
        if e.requests >= REQUESTS_PER_MINUTE {
            return Err(Refused {
                retry_after_s: ((minute + 1) * 60_000 - now).div_euclid(1000).max(1),
            });
        }
        e.requests += 1;
        Ok(())
    }

    /// Records a failure from `ip`.
    pub fn failed(&mut self, ip: IpAddr, now: i64) {
        let e = self.map.entry(who(ip)).or_default();
        e.failures.push_back(now);
        while e.failures.len() > FAILURES_PER_WINDOW {
            e.failures.pop_front();
        }
    }

    fn forget_old(&mut self, now: i64) {
        if self.map.len() < MAX_ADDRESSES {
            return;
        }
        let minute = now.div_euclid(60_000);
        self.map.retain(|_, e| {
            e.minute == minute
                || e.failures
                    .back()
                    .is_some_and(|t| *t > now - FAILURE_WINDOW_MS)
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_requests_is_cut_off_and_the_next_minute_is_fresh() {
        let mut l = Limits::default();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let other: IpAddr = "203.0.113.8".parse().unwrap();
        let t = 1_000_000_000_000;
        for _ in 0..REQUESTS_PER_MINUTE {
            l.request(ip, t).unwrap();
        }
        assert!(l.request(ip, t + 1).is_err());
        assert!(
            l.request(other, t + 1).is_ok(),
            "another address is not affected"
        );
        assert!(l.request(ip, t + 60_000).is_ok());
    }

    #[test]
    fn failures_refuse_the_address_until_they_age_out() {
        let mut l = Limits::default();
        let ip: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let same_64: IpAddr = "2001:db8:1:2::ffff".parse().unwrap();
        let t = 1_000_000_000_000;
        for i in 0..FAILURES_PER_WINDOW {
            l.failed(ip, t + i as i64);
        }
        let r = l.request(same_64, t + 100).unwrap_err();
        assert!(r.retry_after_s > 800, "{r:?}");
        assert!(l
            .request("2001:db8:1:3::1".parse().unwrap(), t + 100)
            .is_ok());
        assert!(l.request(ip, t + FAILURE_WINDOW_MS + 100).is_ok());
    }
}
