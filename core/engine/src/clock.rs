// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where the engine reads the time. Injected so tests never sleep through an Undo window.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of the current time in Unix milliseconds. The queue's delays, back-off and the
/// periodic sync are all measured with it.
pub trait Clock: Send + Sync + 'static {
    /// Now, in Unix milliseconds.
    fn now_ms(&self) -> i64;
}

/// The system's wall clock. Use this in the app.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to. For tests: clones share the same time.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    /// A clock that reads `start_ms` until it is moved.
    pub fn new(start_ms: i64) -> Self {
        Self(Arc::new(AtomicI64::new(start_ms)))
    }

    /// Sets the time.
    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }

    /// Moves the time forward by `ms`.
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_clock_moves_only_when_told_and_clones_share_it() {
        let c = ManualClock::new(1_000);
        let other = c.clone();
        assert_eq!(c.now_ms(), 1_000);
        c.advance(500);
        assert_eq!(other.now_ms(), 1_500);
        other.set(7);
        assert_eq!(c.now_ms(), 7);
    }

    #[test]
    fn the_system_clock_is_after_2020() {
        assert!(SystemClock.now_ms() > 1_577_836_800_000);
    }
}
