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
//! synchronous sections (one queue change, one chunk of at most 500 rows of a sync, one chunk of
//! headers) and never across an `.await`, so it is never held during a network call. A folder sync
//! lets go of the lock and of the thread between its chunks, and reads what is protected (rows
//! with an unfinished operation) again under each chunk's lock, so a UI action taken in between
//! is respected and never waits for a whole folder. Each account has one task that
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
//! gives up after 8 attempts; [`EngineHandle::set_online`]`(true)` ends that back-off. `Auth`
//! pauses the account until [`EngineHandle::signed_in`]; `Permanent` fails the operation and puts
//! the row back. A database error does not make the task spin: it waits `sync_retry_ms`, doubled
//! each time (at most 16 times), before it tries again.
//!
//! # Sending
//!
//! Open the engine with [`Engine::open_with_outbox`] and add each account that can send with
//! [`Engine::add_account_with_submitter`] (a [`croft_protocols::MailSubmitter`], such as
//! [`croft_protocols::SmtpImapSubmitter`]). [`EngineHandle::send`] stores the message in the
//! persisted Outbox (`croft_mail_core::PersistentOutbox`) with its send time at the end of the Undo
//! window and returns an id for the "Sending... Undo" snackbar ([`EngineHandle::undo_send`]);
//! [`EngineHandle::outbox`], `retry_send`, `edit_back_to_draft` and `discard_send` are the Outbox
//! screen. The account task sends what is due, one message at a time and never while offline, then
//! saves the copy in Sent (looking the Message-ID up first, so a crash in between never makes two
//! copies). Screens hear of it as [`Change::Outbox`], [`Change::SendFailed`] and
//! [`Change::NotFiled`].
//!
//! * **Own connection, own lock.** The Outbox lives on a second connection to the same database
//!   file behind its own `Mutex`, never held together with the queue's and never across an
//!   `.await` ([`Engine::open_with_outbox`] says why).
//! * **Never twice.** A message is marked sending on disk before it is handed to the submitter. If
//!   the process dies, the next start fails it as "May not have been sent; check Sent before
//!   retrying" and announces it once. If the send future is dropped while the process lives, the
//!   same happens at once. The engine itself never drops a send: `shutdown` and `remove_account`
//!   wait for it (the submitter's own deadline ends the wait).
//! * **One credential.** A refused sign-in while sending puts the account in the same state as one
//!   from IMAP: [`Change::SignInNeeded`], and nothing of the account (queue, syncs, Outbox) runs
//!   until [`EngineHandle::signed_in`].
//! * **The message has no `Bcc:` or `Resent-Bcc:` header** ([`SendError::BccHeader`], checked with
//!   the SMTP adapter's own line rule, so a lone CR hides nothing); Bcc recipients are only in the
//!   envelope. It has an ASCII Message-ID ([`SendError::NoMessageId`],
//!   [`SendError::MessageIdNotAscii`]).
//! * **A new password** is given with [`EngineHandle::replace_submitter`] and
//!   [`EngineHandle::signed_in`]; an account removed and added back after a refused sign-in starts
//!   waiting for the sign-in again.
//!
//! # Reading
//!
//! [`EngineHandle::fetch_body`] fetches a message's body once (the whole message, through the
//! account's backend, never marking it read), parses and sanitises it with `croft_reader` on a
//! blocking thread, and stores it (`croft_storage::reading`), so it opens offline from then on.
//!
//! # Message-ID lookups never guess before something is destroyed
//!
//! A message that moved (no new UID in the answer) or whose folder was renumbered is found again by
//! its Message-ID, and `MailBackend::find_by_message_id` returns **one** UID. When a destructive
//! operation (delete forever, or a move into Trash or Junk) waits on such a message, the engine
//! gives up instead of re-pointing it when the answer may be the wrong copy: the destination
//! folder has another **local** row with that Message-ID, or two old UIDs of a renumbered folder
//! share a Message-ID. The operation fails with a plain reason. Limit: a duplicate that exists on
//! the server only and has not been synced is invisible to this check, because the backend names
//! one UID. Flag changes are re-pointed as usual.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod body;
mod clock;
mod demo;
mod engine;
mod handle;
mod mime;
mod notify;
mod outbox;
mod search;
mod sync;
mod worker;

pub use body::BodyError;
pub use clock::{Clock, ManualClock, SystemClock};
pub use croft_mail_core::compose;
pub use croft_mail_core::{
    AccountId, NewMessage, Op, OpId, OpKind, OpState, OutboxEntry, OutboxId, OutboxState,
    RemovedMessage, UnreadableOp,
};
pub use demo::{resume_demo, REASON_COPY_NOT_STORED};
pub use engine::{Change, Engine, EngineConfig, EngineError};
pub use handle::{ActionError, EngineHandle, OutboxActionError, SendError};
pub use notify::{NewMail, SyncOnce};
pub use search::SearchError;
pub use worker::{
    MAX_LOOKUPS, REASON_AMBIGUOUS_COPY, REASON_LOST_AFTER_MOVE, REASON_LOST_AFTER_RESET,
};
