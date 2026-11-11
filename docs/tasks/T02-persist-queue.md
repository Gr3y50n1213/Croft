# T02: Persist the operation queue

Crate / folder: `core/mail-core + core/storage`

## Goal
`OpQueue` (core/mail-core/src/ops.rs) lives in memory. Make it survive a kill.

## Spec
- Add an `OpStore` trait in mail-core and an implementation in storage over the `op_queue` table. mail-core must stay free of SQLite.
- Every state change is written in the same transaction as the local optimistic change (for example the `message.is_seen` update plus `message.pending_ops`).
- On start: load unfinished rows, call `recover_after_restart()`.
- Serialise `OpKind` to `payload_json` with a version field.
- Persist the "waiting for a new UID" mark that `confirm_moved(id, None)` creates (a column on the affected op rows, or a small table), in the same transaction as the confirmation. Otherwise a restart would run a later operation against the old UID. The offline flag and the account back-off are not persisted; the engine sets them again on start.

## Done when
- A test enqueues three ops, drops the queue object, reloads it, and gets the same three ops in the same order.
- A test kills the queue mid-`Applying` (simulated) and sees the op `Queued` again after reload.
- Undo inside the window leaves no row in state `queued`; read-then-unread inside the window leaves nothing to send.
- A test confirms a move with no known new UID, drops the queue, reloads it, and the later operation on that message is still held.
