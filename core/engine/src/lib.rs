// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Croft's sync engine: where a person's actions meet the server.
//!
//! It joins the pieces built before it: the server adapter (any [`croft_protocols::MailBackend`]),
//! the sync planner ([`croft_mail_core::plan`]), the persisted operation queue
//! ([`croft_mail_core::PersistentQueue`] over [`croft_storage::SqliteOpStore`]) and the database.
//! `croft-mail-core` stays pure and synchronous; this crate is the only one with both a clock and
//! the network (design note `docs/tasks/T02-T04-design.md`, section 1).
//!
//! # Use
//!
//! ```no_run
//! # use std::sync::Arc;
//! # async fn demo(db: croft_storage::Db, backend: Arc<dyn croft_protocols::MailBackend>) -> Result<(), croft_engine::EngineError> {
//! use croft_engine::{Engine, EngineConfig, SystemClock};
//! let engine = Engine::open(db, Arc::new(SystemClock), EngineConfig::default())?;
//! engine.add_account(1, backend)?; // built by the caller: credentials never pass through here
//! let ui = engine.handle();
//! let mut changes = ui.subscribe();
//! let undo_id = ui.move_to(1, 42, "Archive").ok(); // returns at once
//! # let _ = (undo_id, changes.recv().await);
//! engine.shutdown().await;
//! # Ok(()) }
//! ```
//!
//! # The UI path never waits for the network
//!
//! Every action on [`EngineHandle`] is a plain synchronous call: it looks the message up by its
//! row id (scoped by account), applies the local change and stores the queue row in one SQLite
//! transaction, wakes the account's task and returns (with an [`OpId`] for the Undo snackbar where
//! there is one). Screens observe [`Change`]s from [`EngineHandle::subscribe`] and re-run their
//! queries.
//!
//! # Concurrency and locking
//!
//! The queue and the database live together behind **one** `std::sync::Mutex` (the queue owns the
//! store, which owns the `Db`; rusqlite connections are not `Sync`). It is held only for short
//! synchronous sections (one queue change, one sync transaction, one chunk of headers) and never
//! across an `.await`, so it is never held during a network call. Each account has one task that
//! runs its operations **one at a time**, in order, and syncs its folders between them; different
//! accounts run in parallel. [`EngineHandle::run_once`] takes the same per-account turn, so it
//! never runs alongside the account's task.
//!
//! # Wake-ups
//!
//! An account task sleeps until the earliest of: its next due operation
//! ([`croft_mail_core::OpQueue::next_wake_ms_for`]), its next periodic full sync
//! ([`EngineConfig::sync_interval_ms`]), or a poke (every UI action, [`EngineHandle::sync_now`],
//! [`EngineHandle::set_online`], shutdown). IMAP IDLE is not in the `MailBackend` trait yet; the
//! place to add it is marked in the account loop (`worker.rs`, `run_loop`).
//!
//! # Retries
//!
//! The platform reports connectivity with [`EngineHandle::set_online`]: while offline nothing is
//! tried and no attempts are used. A `Transient` answer from the server goes to the queue's
//! `reject`, which backs the whole account off (5 s, 30 s, 2 min, 10 min, 1 h, then hourly) and
//! gives up after 8 attempts. `Auth` pauses the account until [`EngineHandle::signed_in`];
//! `Permanent` fails the operation and puts the row back.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod clock;
mod engine;
mod handle;
mod sync;
mod worker;

pub use clock::{Clock, ManualClock, SystemClock};
pub use croft_mail_core::{AccountId, Op, OpId, OpKind, OpState, UnreadableOp};
pub use engine::{Change, Engine, EngineConfig, EngineError};
pub use handle::{ActionError, EngineHandle};
pub use worker::{MAX_LOOKUPS, REASON_LOST_AFTER_MOVE, REASON_LOST_AFTER_RESET};
