//! Croft core logic: what to fetch, what to send, in what order, and what to do when it fails.
//!
//! Everything here is pure and synchronous so it can be tested without a network or a database.
//! The async sync engine (added next) drives these pieces:
//!
//! 1. [`sync_plan::plan`] compares the server's folder with the local copy and says what changed.
//! 2. [`ops::OpQueue`] holds the person's actions (mark read, delete, move) until the server
//!    confirms them, with an undo window, ordering per message, retry and backoff.
//! 3. [`persist::PersistentQueue`] keeps that queue in an [`persist::OpStore`] (SQLite, in
//!    `croft-storage`) so it survives a kill, writing each local change and its queue row together.

pub mod ops;
pub mod persist;
pub mod sync_plan;

pub use ops::{
    AccountId, AwaitingUid, CancelError, Enqueued, Location, Op, OpId, OpKind, OpQueue, OpState,
    Rejection,
};
pub use persist::{
    Batch, LocalEffect, OpStore, PersistentQueue, QueueError, StoredQueue, UnreadableOp,
};
pub use sync_plan::{plan, LocalFolder, SyncPlan};
