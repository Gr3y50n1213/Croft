// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Rate limits in memory: a token bucket per key (a device's token, or a client address), in two
//! generations so memory stays bounded and no request scans the table. An entry not seen for an
//! hour is dropped, which loses nothing: its bucket would be full again by then.

use std::collections::HashMap;
use std::net::IpAddr;

/// Keys remembered per generation before the older generation is dropped.
const MAX_KEYS: usize = 100_000;
/// How long a generation lasts.
const GENERATION_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: i64,
}

/// A rate limit: `per_hour` on average, at most `burst` at once.
#[derive(Debug)]
pub struct Limits {
    per_ms: f64,
    burst: f64,
    current: HashMap<u128, Bucket>,
    previous: HashMap<u128, Bucket>,
    since: i64,
}

impl Limits {
    /// A limit of `per_hour` on average, `burst` at once.
    pub fn new(per_hour: u32, burst: u32) -> Self {
        Self {
            per_ms: f64::from(per_hour) / GENERATION_MS as f64,
            burst: f64::from(burst.max(1)),
            current: HashMap::new(),
            previous: HashMap::new(),
            since: 0,
        }
    }

    /// Takes one from `key`'s bucket at `now` (milliseconds); `false` when it is empty.
    pub fn take(&mut self, key: u128, now: i64) -> bool {
        if !self.current.contains_key(&key)
            && (now - self.since >= GENERATION_MS || self.current.len() >= MAX_KEYS)
        {
            self.previous = std::mem::take(&mut self.current);
            self.since = now;
        }
        let burst = self.burst;
        let bucket = match self.previous.remove(&key) {
            Some(b) => self.current.entry(key).or_insert(b),
            None => self.current.entry(key).or_insert(Bucket {
                tokens: burst,
                at: now,
            }),
        };
        let elapsed = (now - bucket.at).max(0) as f64;
        bucket.tokens = (bucket.tokens + elapsed * self.per_ms).min(burst);
        bucket.at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The key a client address counts under: IPv4 (also when mapped into IPv6) whole, IPv6 by its
/// /64 (one host usually has a whole /64).
pub fn who(ip: IpAddr) -> u128 {
    match ip.to_canonical() {
        IpAddr::V4(a) => u128::from(u32::from(a)),
        IpAddr::V6(a) => (u128::from(a) >> 64) | (1 << 127),
    }
}

/// The key a device's token counts under (whichever endpoint of it is used).
pub fn token_key(token: &str) -> u128 {
    let digest = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());
    let mut first = [0u8; 16];
    first.copy_from_slice(&digest.as_ref()[..16]);
    u128::from_be_bytes(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_then_the_average_rate() {
        let mut l = Limits::new(60, 10);
        for _ in 0..10 {
            assert!(l.take(1, 0));
        }
        assert!(!l.take(1, 0));
        assert!(l.take(2, 0), "another key has its own bucket");
        // One a minute comes back.
        assert!(!l.take(1, 59_000));
        assert!(l.take(1, 60_500));
        assert!(!l.take(1, 61_000));
        // Over a generation change the bucket is kept.
        for k in 3..10 {
            assert!(l.take(k, GENERATION_MS + 1));
        }
        assert!(l.take(1, GENERATION_MS + 2), "refilled after an hour");
    }

    #[test]
    fn addresses_count_by_host_and_tokens_by_hash() {
        let a: IpAddr = "2001:db8::1".parse().unwrap_or(IpAddr::from([0u8; 4]));
        let b: IpAddr = "2001:db8::2".parse().unwrap_or(IpAddr::from([0u8; 4]));
        assert_eq!(who(a), who(b));
        assert_ne!(
            who(IpAddr::from([10, 0, 0, 1])),
            who(IpAddr::from([10, 0, 0, 2]))
        );
        assert_eq!(token_key("x"), token_key("x"));
        assert_ne!(token_key("x"), token_key("y"));
    }
}
