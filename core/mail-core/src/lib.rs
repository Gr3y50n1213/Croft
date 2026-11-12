//! Croft core logic: what to fetch, what to send, in what order, and what to do when it fails.
//!
//! Everything here is pure and synchronous so it can be tested without a network or a database.
//! The async sync engine (`croft-engine`, in `core/engine`) drives these pieces:
//!
//! 1. [`sync_plan::plan`] compares the server's folder with the local copy and says what changed.
//! 2. [`ops::OpQueue`] holds the person's actions (mark read, delete, move) until the server
//!    confirms them, with an undo window, ordering per message, retry and backoff.
//! 3. [`persist::PersistentQueue`] keeps that queue in an [`persist::OpStore`] (SQLite, in
//!    `croft-storage`) so it survives a kill, writing each local change and its queue row together.
//! 4. [`outbox::Outbox`] holds sent-but-not-yet-delivered mail (Undo send, retry, never twice), and
//!    [`outbox_store::PersistentOutbox`] keeps it in an [`outbox_store::OutboxStore`].

pub mod compose;
pub mod notify;
pub mod ops;
pub mod outbox;
pub mod outbox_store;
pub mod persist;
pub mod search;
pub mod sync_plan;

pub use ops::{
    AccountId, AwaitingUid, CancelError, Enqueued, Location, Op, OpId, OpKind, OpQueue, OpState,
    Rejection,
};
pub use outbox::{
    ActionError, FilingRejection, NewMessage, Outbox, OutboxEntry, OutboxId, OutboxState,
    SendRejection, UndoError,
};
pub use outbox_store::{
    DueMessage, OutboxBatch, OutboxError, OutboxStore, PersistentOutbox, RemovedMessage,
    StoredOutbox,
};
pub use persist::{
    Batch, FolderReset, LocalEffect, MovedOut, OpStore, PersistentQueue, QueueError, StoredQueue,
    UnreadableOp,
};
pub use sync_plan::{plan, LocalFolder, SyncPlan};
