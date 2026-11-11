# Design: persisting the queue (T02) and following a message after a move (T04)

Written by the orchestrator on 11 November 2026, before T02 and T04 were built. The master prompt keeps
these calls for the orchestrator: the database migration, and how queued operations follow a message
after a move. T02 and T04 are built to this note. If the code has to differ, update this note in the same commit.

## 1. Where the code lives

`CLAUDE.md` says `core/mail-core` is pure and synchronous (no clock, no I/O). T04 asks for a tokio task
"in core/mail-core". `CLAUDE.md` wins (master prompt section 6), so:

- `core/mail-core` keeps the pure pieces: `OpQueue`, `sync_plan`, and, new in T02, the `OpStore` trait and
  the `LocalEffect` vocabulary. No SQLite, no tokio.
- `core/storage` implements `OpStore` over SQLite.
- A new crate **`core/engine`** (`croft-engine`) holds the async sync engine from T04. It depends on
  mail-core, storage and protocols. It is the only crate that has both a clock and the network.

## 2. Migration 0002 (T02)

`core/storage/migrations/0002_queue_persistence.sql`:

```sql
-- While a move or delete is waiting (Undo window, or not yet confirmed), the message is hidden from
-- lists but kept. Its row is deleted only once the server confirms it is gone (never delete locally first).
ALTER TABLE message ADD COLUMN local_hidden INTEGER NOT NULL DEFAULT 0;

-- Messages that were moved on the server but whose new UID is not known yet (no UIDPLUS, or the
-- server's answer did not include it). Later operations on the old address wait until the engine
-- finds the new UID by Message-ID (then repoints them) or gives up (then they fail).
CREATE TABLE op_awaiting_uid (
    account_id      INTEGER NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    folder          TEXT    NOT NULL,     -- old folder path
    uid             INTEGER NOT NULL,     -- old UID
    dest_folder     TEXT    NOT NULL,     -- where it was moved to
    message_id_hdr  TEXT,                 -- to find it again; NULL means it cannot be found
    lookups         INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (account_id, folder, uid)
);
```

The list query (`messages in folder, newest first`) also excludes `local_hidden = 1`.

## 3. The OpStore contract (T02)

- `OpQueue` stays the single source of the queue's rules. It gains read access to the awaiting set (with
  the destination and Message-ID: `confirm_moved` keeps them) and a constructor that rebuilds the queue from
  stored ops and awaiting entries, setting the next id to the largest stored id plus one.
- `OpStore` (trait in mail-core) has two calls: `load()` returns unfinished ops and awaiting entries, and
  `commit(batch)` writes one batch atomically. A batch holds the ops whose rows changed (insert or update by id),
  the awaiting entries added or removed, and zero or more `LocalEffect`s.
- `LocalEffect` is the optimistic change to the person's view, applied in the **same transaction** as the
  queue rows:
  - `SetSeen` / `SetFlagged { account, folder, uid, value }`: update the message row.
  - `Hide { account, folder, uid }` / `Unhide`: set or clear `local_hidden` (move, delete forever).
  - `PendingDelta { account, folder, uid, delta }`: add to `message.pending_ops`. +1 when an op is queued,
    -1 when it finishes (confirmed, failed, cancelled). Never below 0.
  - `Restore`: a cancelled or failed flag op puts back the previous value; a cancelled or failed move unhides.
  - `RemoveMessage { account, folder, uid }`: only after the server confirmed a move or delete.
  - `CopyMessage { account, from: (folder, uid), to: (folder, uid) }`: after a confirmed move with a known new
    UID, the row is copied to the destination folder (the copy keeps the pending count of the operations
    that were repointed to it), then the old row is removed. This makes the moved copy visible in
    Archive at once and gives later operations (a star) a row to change.
- A small `PersistentQueue<S: OpStore>` in mail-core wraps `OpQueue` + store: every public method calls
  the in-memory queue, works out the batch, and commits it. If the commit fails, the in-memory change is
  rolled back (or the queue is reloaded) so memory never runs ahead of the disk.
- `payload_json` is `{"v":1, ...}` with the `OpKind` fields; `kind` column holds `set_seen`, `set_flagged`,
  `move`, `delete_forever`. An unknown `v` fails that row with a clear reason instead of guessing.
- Offline flags, pauses and account back-off are not stored; the engine sets them again on start.
- Finished rows may be deleted by `purge_finished`. Ids can then be reused only for ops created after the
  purge; the UI only holds the id of the newest op for its Undo snackbar, so this is safe. (Decision recorded.)

## 4. Following a message after a move (T04)

A message's address is (folder, UID, UIDVALIDITY). Moving changes it. Operations queued later for the same
message were recorded against the old address, and `OpQueue` already holds them back behind the move
(`blocked_by_earlier`). The engine does this when the move's answer comes back:

1. **Server gave the new UID** (COPYUID in `MoveOutcome.dest_uids`): `confirm_moved(id, Some((dest, new_uid)))`.
   In the same transaction: `CopyMessage` old → new, `RemoveMessage` old, store the repointed ops. The star
   queued after the archive now runs against the copy in Archive.
2. **No new UID** (no UIDPLUS, or the old UID was already gone because this is a repeat after a crash):
   `confirm_moved(id, None)`; the awaiting entry is stored with the destination and the Message-ID from the
   local row; the old row is removed only after the lookup below, so the person's change is not lost.
   Then, straight away and again on each later sync of the destination (up to 3 lookups):
   `find_by_message_id(dest, message_id)`. Found → `repoint` + `CopyMessage` + `RemoveMessage`.
   Not found after 3 lookups, or no Message-ID at all → `abandon(account, old, reason)`: the later ops fail with
   "Couldn't find the message after moving it", the old row is removed, and the next sync of the destination
   shows the message with the server's flags.
3. **UIDVALIDITY changed** for a folder (`sync_plan` returns `reset`): before dropping the folder's rows,
   collect every unfinished op whose target is in that folder, with the Message-ID of its local row. Drop the
   rows, store the new UIDVALIDITY, re-sync, then for each distinct old UID: `find_by_message_id(folder, id)` →
   `repoint` (and the new row gets `pending_ops` set), or `abandon`. Awaiting entries whose destination is this
   folder are looked up the same way.
4. **Repeat after a crash.** Every command is safe to run twice: a flag set twice is harmless; a MOVE whose
   source UID is gone returns no mapping for it, which lands in case 2; a delete-forever of a missing UID is a no-op.

The engine runs operations for one account one at a time in this first version. The contract allows up
to four messages at once per account; that is a later speed-up and does not change the rules above.

## 5. The UI path (T04)

The UI never waits for the network. A person's action calls the engine handle, which in one SQLite transaction
applies the `LocalEffect`s and stores the op, then wakes the account's task (`tokio::sync::Notify`). Screens
observe a `tokio::sync::broadcast` of changed folder ids and re-run their queries.

## 6. What T02 changed from this note (recorded by the T02 build)

- **No separate `Restore` effect.** A cancelled or failed flag op is `SetSeen` / `SetFlagged` with the value to put back,
  and a cancelled or failed move or delete is `Unhide`. The value is the one of the latest unfinished op on the same
  flag of the same message if there is one, otherwise the opposite of what the finished op set. (The note's plain
  "previous value" is wrong when a second change is made while the first is in flight.)
- **Batch and load.** `Batch` also has `purged` (finished rows to delete). `load()` returns `StoredQueue` with `max_id`
  (the largest id ever stored, so an id is never reused after finished rows are discarded) and `unreadable` rows. It
  loads unfinished rows only and deletes finished ones, because nothing reads them after a restart.
- **JSON lives in storage** (`core/storage/src/op_store.rs`), so mail-core has no serde. A row whose `v` is not 1, or
  that cannot be read, is marked `failed` with a plain reason, its `pending_ops` is released and a hidden message is
  shown again (using `target_key`, which is `<uid>@<folder path>` whatever the payload version). It is reported by
  `PersistentQueue::unreadable()`.
- **Awaiting entry.** `AwaitingUid` (dest, Message-ID, lookups, created time) is stored in `op_awaiting_uid`.
  `confirm_moved_with(id, loc, message_id, now_ms)` is the call the engine should use; `confirm_moved(id, loc)` keeps
  working and stores no Message-ID. `record_lookup` counts the lookups from section 4.
- **UIDVALIDITY reset (section 4, case 3).** After a reset the old address may belong to a different message, so
  `abandon` (which removes the old row) must not be used there. Use `repoint_after_reset` (puts `pending_ops` and the
  local flag or hidden state on the new row) and `abandon_after_reset` (fails the ops, touches no row).
- **Plain `confirm` of a move** removes the old row and nothing follows it. The engine should use `confirm_moved*`.
- **Folder counters and hidden rows (fixed by the orchestrator).** The 0001 triggers counted a hidden row until it was
  removed, so a folder's unread badge dropped only when the server confirmed an archive. Migration
  `0003_counters_skip_hidden.sql` replaces the counter triggers: a hidden row counts towards neither total nor unread,
  one UPDATE that changes both `is_seen` and `local_hidden` is counted once, and rows already hidden at upgrade time are
  taken out once.
