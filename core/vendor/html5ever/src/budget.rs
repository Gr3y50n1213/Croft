// CROFT-PATCH: this whole file is new (see CROFT-PATCHES.md, change 3). Same licence as the
// rest of the crate (MIT OR Apache-2.0).

//! A per-thread **work budget** for one parse.
//!
//! The HTML5 tree builder has rules whose cost no byte scan of the input can predict for every
//! input (nested scope checks, rebuilt formatting elements, foster parenting). So the parser
//! measures instead: every token, every step of every walk over the stack of open elements or the
//! list of active formatting elements, every node created and every attribute byte copied into a
//! rebuilt element is **charged** here. Once the budget is spent (or the caller cancels), the
//! tree builder ignores further tokens and the tokenizer drops the rest of its input, so a parse
//! ends within a small, fixed amount of work whatever the input.
//!
//! The budget lives in a thread local because the parser runs on its own thread and the charge
//! points sit deep in code that has no way to pass it along. With no budget installed on the
//! thread, every charge succeeds (the stock behaviour).
//!
//! ```
//! use html5ever::budget;
//! let guard = budget::install(budget::Limits { units: 100, bytes: 10 }, None);
//! assert!(budget::charge(60));
//! assert!(!budget::charge(60));
//! assert_eq!(budget::stopped(), Some(budget::Stop::Exhausted));
//! drop(guard);
//! assert!(budget::charge(1_000));
//! ```

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Units charged for every node created (element, comment, text node), on top of one unit per
/// attribute. A node and its attribute vector take a few hundred bytes; this keeps the memory a
/// budget allows in proportion to its time.
pub const NODE_UNITS: u64 = 50;

/// Units charged per attribute of a created element (the attribute list is copied and later
/// filtered attribute by attribute).
pub const ATTRIBUTE_UNITS: u64 = 40;

/// Units charged per character reference (and per bare `&`, which starts one): the tokenizer's
/// reference matcher allocates and can push characters back.
pub const CHAR_REF_UNITS: u64 = 60;

/// Units charged per parse error the tokenizer reports (it formats a message for each).
pub const ERROR_UNITS: u64 = 10;

/// Units charged per attribute when the tree builder compares two formatting elements' attribute
/// lists (it copies and sorts both).
pub const COMPARE_UNITS_PER_ATTRIBUTE: u64 = 12;

/// Units charged per URL a sanitiser parses (an allowed `href` or `src`), on top of one unit per
/// [`URL_BYTES_PER_UNIT`] bytes.
pub const URL_UNITS: u64 = 120;

/// Bytes of a URL per unit charged when it is parsed.
pub const URL_BYTES_PER_UNIT: u64 = 2;

/// Elements moved in one `Vec` shift (an insert or remove in the middle of a list) per unit
/// charged.
pub const SHIFT_PER_UNIT: u64 = 64;

/// Units charged per byte of a `style` attribute each time a CSS filter reads it (the CSS
/// tokenizer is the slowest per byte of the steps that run per attribute).
pub const CSS_UNITS_PER_BYTE: u64 = 2;

/// How many charges pass between two reads of the cancel flag.
const CHECK_EVERY: u32 = 1024;

/// How much work one parse may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Work units: tokens, walk steps, nodes (see the module documentation).
    pub units: u64,
    /// Bytes of attributes copied into rebuilt elements.
    pub bytes: u64,
}

/// Why the work stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The budget was spent: the input is too complex. The same input always stops here.
    Exhausted,
    /// The caller set the cancel flag (a deadline passed).
    Cancelled,
}

struct State {
    active: Cell<bool>,
    units: Cell<u64>,
    bytes: Cell<u64>,
    limits: Cell<Limits>,
    stop: Cell<Option<Stop>>,
    until_check: Cell<u32>,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
}

thread_local! {
    static STATE: State = const {
        State {
            active: Cell::new(false),
            units: Cell::new(0),
            bytes: Cell::new(0),
            limits: Cell::new(Limits { units: 0, bytes: 0 }),
            stop: Cell::new(None),
            until_check: Cell::new(0),
            cancel: RefCell::new(None),
        }
    };
}

/// Removes the budget from this thread when dropped. Not `Send`: it belongs to the thread that
/// installed it.
#[must_use = "the budget is removed when the guard is dropped"]
pub struct Guard {
    _not_send: PhantomData<*const ()>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = STATE.try_with(|s| {
            s.active.set(false);
            s.stop.set(None);
            s.cancel.replace(None);
        });
    }
}

/// Installs a budget on this thread (replacing any earlier one) until the guard is dropped.
/// `cancel`, when given, stops the work soon after it is set to true (it is read every
/// 1024 charges, so within microseconds of parser work).
pub fn install(limits: Limits, cancel: Option<Arc<AtomicBool>>) -> Guard {
    let _ = STATE.try_with(|s| {
        s.active.set(true);
        s.units.set(limits.units);
        s.bytes.set(limits.bytes);
        s.limits.set(limits);
        s.stop.set(None);
        s.until_check.set(0);
        s.cancel.replace(cancel);
    });
    Guard {
        _not_send: PhantomData,
    }
}

fn stop(s: &State, why: Stop) -> bool {
    if s.stop.get().is_none() {
        s.stop.set(Some(why));
    }
    false
}

/// Charges `units` of work. Returns whether to go on: false once the budget is spent or the work
/// was cancelled (and from then on for every later charge).
#[inline]
pub fn charge(units: u64) -> bool {
    STATE
        .try_with(|s| {
            if !s.active.get() {
                return true;
            }
            if s.stop.get().is_some() {
                return false;
            }
            let left = s.units.get();
            if units > left {
                s.units.set(0);
                return stop(s, Stop::Exhausted);
            }
            s.units.set(left - units);
            let until = s.until_check.get();
            if until == 0 {
                s.until_check.set(CHECK_EVERY);
                let cancelled = s
                    .cancel
                    .borrow()
                    .as_ref()
                    .is_some_and(|c| c.load(Ordering::Relaxed));
                if cancelled {
                    return stop(s, Stop::Cancelled);
                }
            } else {
                s.until_check.set(until - 1);
            }
            true
        })
        .unwrap_or(true)
}

/// Charges `bytes` copied into a rebuilt element. Returns whether to go on, like [`charge`].
#[inline]
pub fn charge_bytes(bytes: usize) -> bool {
    STATE
        .try_with(|s| {
            if !s.active.get() {
                return true;
            }
            if s.stop.get().is_some() {
                return false;
            }
            let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
            let left = s.bytes.get();
            if bytes > left {
                s.bytes.set(0);
                return stop(s, Stop::Exhausted);
            }
            s.bytes.set(left - bytes);
            true
        })
        .unwrap_or(true)
}

/// Charges a shift of `moved` elements in a list (a `Vec` insert or remove in the middle).
#[inline]
pub fn charge_shift(moved: usize) -> bool {
    charge(1 + u64::try_from(moved).unwrap_or(u64::MAX) / SHIFT_PER_UNIT)
}

/// Why the work stopped, if it did. `None` with no budget installed.
pub fn stopped() -> Option<Stop> {
    STATE
        .try_with(|s| if s.active.get() { s.stop.get() } else { None })
        .unwrap_or(None)
}

/// True once the work stopped (spent or cancelled). Cheap: called once per tokenizer step.
#[inline]
pub fn spent() -> bool {
    STATE
        .try_with(|s| s.active.get() && s.stop.get().is_some())
        .unwrap_or(false)
}

/// The work charged so far as `(units, bytes)`, for calibration and tests. `(0, 0)` with no
/// budget installed.
pub fn used() -> (u64, u64) {
    STATE
        .try_with(|s| {
            if !s.active.get() {
                return (0, 0);
            }
            let limits = s.limits.get();
            (
                limits.units.saturating_sub(s.units.get()),
                limits.bytes.saturating_sub(s.bytes.get()),
            )
        })
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges_stop_at_the_limit_and_stay_stopped() {
        let _g = install(Limits { units: 10, bytes: 5 }, None);
        assert!(charge(10));
        assert!(!charge(1));
        assert!(spent());
        assert!(!charge(0), "stays stopped");
        assert_eq!(stopped(), Some(Stop::Exhausted));
    }

    #[test]
    fn bytes_have_their_own_limit() {
        let _g = install(Limits { units: 10, bytes: 5 }, None);
        assert!(charge_bytes(5));
        assert!(!charge_bytes(1));
        assert_eq!(stopped(), Some(Stop::Exhausted));
    }

    #[test]
    fn the_cancel_flag_stops_the_work() {
        let flag = Arc::new(AtomicBool::new(false));
        let _g = install(Limits { units: u64::MAX, bytes: 0 }, Some(flag.clone()));
        assert!(charge(1));
        flag.store(true, Ordering::Relaxed);
        let mut n = 0;
        while charge(1) {
            n += 1;
        }
        assert!(n <= CHECK_EVERY as usize);
        assert_eq!(stopped(), Some(Stop::Cancelled));
    }

    #[test]
    fn without_a_budget_everything_goes() {
        assert!(charge(u64::MAX));
        assert!(charge_bytes(usize::MAX));
        assert!(!spent());
        assert_eq!(stopped(), None);
    }
}
