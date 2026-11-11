//! Croft core logic: what to fetch, what to send, in what order, and what to do when it fails.
//!
//! Everything here is pure and synchronous so it can be tested without a network or a database.
//! The async sync engine (added next) drives these pieces:
//!
//! 1. [`sync_plan::plan`] compares the server's folder with the local copy and says what changed.
//! 2. [`ops::OpQueue`] holds the person's actions (mark read, delete, move) until the server
//!    confirms them, with an undo window, ordering per message, retry and backoff.

pub mod ops;
pub mod sync_plan;

pub use ops::{Enqueued, Op, OpId, OpKind, OpQueue, OpState, Rejection};
pub use sync_plan::{plan, LocalFolder, SyncPlan};
