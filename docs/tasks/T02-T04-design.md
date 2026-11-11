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
  and `unreadable` rows. It loads unfinished rows only and deletes finished ones, because nothing reads them after a
  restart (unreadable rows excepted, see below).
- **Ids are never reused (review fix).** This replaces the "ids can be reused after a purge" decision in section 3. The
  largest id ever written is kept in the app-wide `setting` table, key `op_queue.max_id` (a JSON number, no migration),
  and is raised in the same transaction as the insert of an op with a larger id. `load()` returns
  `max(that value, MAX(id) of op_queue)`. `MAX(id)` alone shrinks when rows are purged or deleted on load.
- **Hidden copies and unhide (review fix).** After a confirmed move the copy at the new address is shown, unless an
  unfinished `MoveTo` or `DeleteForever` was re-pointed at it: then `CopyMessage` is followed by `Hide` (an existing
  destination row is no longer forced visible). A failed or cancelled move or delete emits `Unhide` only when no other
  unfinished `MoveTo` or `DeleteForever` targets the same address, the same way flag restores look at later flag ops.
- **`repoint` to its own address (review fix)** returns 0 and writes nothing. (`repoint_after_reset` may legitimately
  keep the same address: the new row after the reset needs its pending count.)
- **JSON lives in storage** (`core/storage/src/op_store.rs`), so mail-core has no serde. A row whose `v` is not 1, or
  that cannot be read, is marked `failed` with a plain reason, its `pending_ops` is released and a hidden message is
  shown again (using `target_key`, which is `<uid>@<folder path>` whatever the payload version). This is done once, in
  the load that first meets the row. It is reported by `PersistentQueue::unreadable()`.
  **Review fix:** the row is not deleted by later loads. A failed row whose `last_error` is one of the two unreadable
  reasons is kept and listed again in `unreadable` at every load, until the engine has told the person and calls
  `PersistentQueue::acknowledge_unreadable(ids)`, which deletes them in one batch (a kill in between loses nothing).
  A damaged row with no usable `target_key` releases nothing, because the message is unknown, but is reported. Its
  message may stay hidden or keep a stale `pending_ops`. **No code path clears that on sync** (checked: `messages::upsert`
  never touches `local_hidden` or `pending_ops`, and nothing else writes them outside the queue), so it would stay stuck.
  **T04 must therefore, during a full folder sync, set `local_hidden = 0` and `pending_ops = 0` on every row of that
  folder that has no unfinished op in the queue** (the queue's `pending_uids` says which have one). That also repairs rows
  left over by any other bug.
- **Awaiting entry.** `AwaitingUid` (dest, Message-ID, lookups, created time) is stored in `op_awaiting_uid`.
  `confirm_moved_with(id, loc, message_id, now_ms)` is the call the engine should use; `confirm_moved(id, loc)` keeps
  working and stores no Message-ID. `record_lookup` counts the lookups from section 4.
- **UIDVALIDITY reset (section 4, case 3).** After a reset the old address may belong to a different message, so
  `abandon` (which removes the old row) must not be used there. Which call the engine uses:
  - Ops whose address is in the **reset folder**, no awaiting entry: `repoint_after_reset` (puts `pending_ops` and the
    local flag or hidden state on the new row) or `abandon_after_reset` (fails the ops, touches no row).
  - An **awaiting-UID entry** (a confirmed move with no new UID) whose destination folder was reset: the old address is in
    the move's source folder, which was not reset, so its hidden row still exists. Use the same two `_after_reset` calls:
    when the address has an awaiting entry they also resolve the entry and remove the old hidden row (its pending count
    goes with it; `repoint_after_reset` puts the count and the local flag values on the new row). `repoint` and `abandon`
    (the non-reset forms) are for the same awaiting entry when no reset is involved: they copy or remove the row as in
    section 4, case 2.
  - If the **source** folder of an awaiting entry is the one that was reset, its old row was dropped with the folder and
    the old address may be another message now. The `_after_reset` calls would remove that other row. In that case the
    engine should not resolve the entry with them until the folder has been synced again and the row at the old
    address is known to be gone or hidden. T02 does not guard this: T04 decides it, and it is rare (a move out of a folder
    whose UIDVALIDITY changes before the new UID was found).
- **Plain `confirm` of a move** removes the old row and nothing follows it. The engine should use `confirm_moved*`.
- **Folder counters and hidden rows (fixed by the orchestrator).** The 0001 triggers counted a hidden row until it was
  removed, so a folder's unread badge dropped only when the server confirmed an archive. Migration
  `0003_counters_skip_hidden.sql` replaces the counter triggers: a hidden row counts towards neither total nor unread,
  one UPDATE that changes both `is_seen` and `local_hidden` is counted once, and rows already hidden at upgrade time are
  taken out once.

## 7. What T04 changed from this note (recorded by the T04 build)

The engine is `core/engine` (`croft-engine`), as section 1 says. Its public API is `Engine` (open, `add_account`
with a task, `attach_account` without one, `remove_account`, `shutdown`), `EngineHandle` (the UI side, below),
`Change`, `Clock` / `SystemClock` / `ManualClock`, `EngineConfig`, `ActionError`, `EngineError`.

- **Locking (decision).** The queue owns the store, which owns the `Db`; all three sit behind one `std::sync::Mutex`
  shared by the account tasks and the UI handle. It is held only for short synchronous sections (one queue change, one
  sync transaction, one chunk of headers) and never across an `.await`, so never during a network call. Sync writes go
  through `PersistentQueue::store_mut()` → `SqliteOpStore::db_mut()` → `Db::transaction`.
- **One operation at a time per account (decision, as in section 4).** Different accounts run in parallel.
  `EngineHandle::run_once` (background jobs, tests) takes the same per-account turn as the task.
- **The queue is shared by all accounts**, so each account task uses the new `take_due_for` / `next_wake_ms_for`.
  `next_wake_ms` (both forms) now skips an operation held behind an earlier unfinished one on the same message: it
  reported a time in the past that `take_due` then refused, and a task sleeping until the next wake would spin until
  the earlier one (an archive in its Undo window) ran.
- **UIDVALIDITY reset is one transaction, not the `_after_reset` calls of section 6.** New
  `PersistentQueue::reset_folder(account, &FolderReset)` writes, in one batch: the folder's rows cleared, the fresh
  headers of the matched messages, every re-pointed operation's pending count and local value on its new row, the
  folder's new UIDVALIDITY (alone, see the review fixes below), and the failed operations. Three `LocalEffect`s were added for it (`ClearFolder`,
  `PutHeader`, `SetFolderStatus`). Why: an operation's UID means something only with the stored UIDVALIDITY. With
  separate commits, a kill between re-pointing operations and storing the new UIDVALIDITY makes the next start read
  UIDs in the wrong numbering and act on another message. All Message-ID lookups and header fetches happen first, with
  nothing changed; a failure leaves the folder as it was and the next sync starts the reset again. Re-pointing uses the
  new `OpQueue::repoint_many`, because a renumbered folder often maps one message's new UID onto another's old UID
  (per-pair `repoint` would move such operations twice). `repoint_after_reset` / `abandon_after_reset` still exist; the
  engine no longer uses them.
- **Awaiting entry whose source folder is reset (section 6 left this to T04).** Resolved inside the reset and never left
  waiting: looked up in its destination by the stored Message-ID before anything changes; found → re-pointed there (the
  destination row is written from its fetched header in the same batch); not found → its operations fail. The old row
  went with the folder, and nothing is ever written at the old address.
- **Awaiting entry whose destination folder is reset.** Found there → re-pointed in the reset batch. Not found → left
  waiting and counted by the usual lookups (up to 3), not failed at once: its old hidden row is in the source folder,
  which was not reset, so waiting is safe.
- **Folder check before every operation (new).** Before an operation is sent, the UIDVALIDITY of its folder and, for a
  move, of the destination is read (`folder_status`) and compared with the stored one; if it changed, that folder is
  synced (reset) first and the operation continues as re-pointed, or fails. Without it, an operation queued before the
  server renumbered its folder, and sent before the next sync noticed, goes to whichever message now has the old UID;
  and a move into a renumbered destination gets a new UID in the new numbering, so `CopyMessage` would write the moved
  copy over the stale local row with that UID and later operations would follow the wrong Message-ID. For the same
  reason, awaiting entries are not resolved into a folder whose reset was put off. Cost: one STATUS per folder per
  operation.
  A small window remains between that STATUS and the STORE/MOVE: closing it needs the adapter to check UIDVALIDITY when
  it selects the folder for a write, which `MailBackend` does not offer (reported, not changed: `core/protocols` is
  another task's).
- **Lookups.** The lookup straight after a move without a new UID counts as the first of the three. A `Transient` error
  on a lookup is not counted; a `Permanent` one (folder gone) counts as "not found".
- **Sync details.** Rows protected from server flags, removal and the section 6 release are the queue's
  `pending_uids` plus the old addresses of awaiting entries (their rows are kept until the lookup). The protected set is read
  again under the lock of every chunk the plan is applied in (see the review fixes below), so an action taken while a
  snapshot was on its way, or between two chunks, is never overwritten. A snapshot is skipped when CONDSTORE says nothing changed (same UIDVALIDITY, UIDNEXT, HIGHESTMODSEQ).
  Folders the server no longer lists are not deleted locally yet.
- **Retries.** The engine cannot tell a dropped connection from a busy server (both are `Transient`): the platform's
  connectivity callback calls `EngineHandle::set_online`; any `Transient` while online goes to `reject`.
- **Finished operations** are purged at the start of each full sync, so `EngineHandle::operation(id)` shows a failed or
  confirmed one until then; failures are also announced as `Change::OperationFailed`.
- **IDLE** is not in `MailBackend`; the account loop wakes on the next due operation, a periodic full sync
  (`EngineConfig::sync_interval_ms`, 15 min) and a `Notify` poked by every UI action. The place to add IDLE is marked in
  `worker.rs` (`run_loop`).

### Review fixes to the engine (second pass over T04)

- **An interrupted reset no longer hides a folder (high).** The reset batch used to store the server's whole new status
  (UIDVALIDITY, UIDNEXT, HIGHESTMODSEQ) before the headers were fetched. If the fetch was cut short, a CONDSTORE server
  with an unchanged HIGHESTMODSEQ made every later sync skip the snapshot, and the folder stayed empty. Now
  `reset_folder` stores **only the new UIDVALIDITY** (new effect `LocalEffect::SetFolderValidity`, which replaces
  `SetFolderStatus`; `FolderReset` has `uid_validity` instead of `status` and `synced_at_ms`), and sets `uid_next` and
  `highest_modseq` to NULL and leaves `last_synced_at` alone. The full status is written only at the end of a completed
  `apply_snapshot`, as on the normal path. A second guard in `unchanged()`: the number of local rows must equal the
  server's `exists`, so a folder with missing rows is never taken for up to date, whatever caused it.
- **Operations in flight when the task stops (medium).** `OpQueue::recover_applying(account)` and
  `PersistentQueue::recover_applying` put the account's `Applying` operations back to `Queued` in one batch. It is
  called (a) by `remove_account` and `shutdown` after the task was joined or aborted, unless a `run_once` still holds
  the account's turn, (b) by `apply` when a step fails after the operation was handed out (the database refused to
  record the answer, the folder check failed on a storage error, ...), and (c) at the start of every cycle, under the
  account's turn, when nothing of the account can be in flight; (c) also covers a failure of (a) or (b) to write. The
  attempt that was already counted stays counted.
- **Short lock holds (medium).** A folder sync reads the folder's rows once and works out the plan with no lock held.
  Server flags and removals are then applied in chunks of 500 rows, one lock hold and one transaction each, and the
  protected set (pending UIDs plus awaiting entries) is read again under each chunk's lock, so an action taken between
  chunks is respected (the SQL of `set_server_flags` also keeps local `is_seen`/`is_flagged` while `pending_ops > 0`, but
  `delete_uids` has no such guard). The task yields to the runtime between chunks. A reset deletes the folder's rows in
  chunks **before** its final batch, sparing rows with an unfinished operation (their Message-IDs are needed); the
  final batch is then only the UIDVALIDITY change, the re-pointed operations and the few rows they need, and its
  `ClearFolder` removes what is left. A reset cut short between chunks leaves the old UIDVALIDITY stored, so the next
  sync starts it again; the rows already deleted were going to be deleted. The lock is a std `Mutex` and is not fair, so
  the yield between chunks is what gives a waiting UI thread its turn in practice; there is no hard bound on the wait
  beyond one chunk's transaction.
- **A failing database does not spin the task (low).** After a `Storage` error the account task waits `sync_retry_ms`,
  doubled for each further failure in a row (at most 16 times), before its next attempt; a poke from the UI still
  starts an attempt at once. Other errors are as before (the queue's own back-off covers them).
- **Coming back online ends the back-off (low).** `set_online(true)` calls `PersistentQueue::clear_hold(account, now)`:
  the account's hold is removed and so is the retry time of each queued operation that has already been tried (an
  operation never tried keeps its Undo window or coalescing delay). Attempts stay counted.
- **Message-ID lookups never guess before something is destroyed (low).** For `DeleteForever` and a `MoveTo` into a
  Trash or Junk folder waiting on a message found by Message-ID: (1) after a move without a new UID, if the destination
  has another **local** row with that Message-ID (any row but the found UID), the operations on the message are abandoned
  with `REASON_AMBIGUOUS_COPY`; (2) in a reset, if the old folder has two rows with that Message-ID, or two old UIDs
  would map to one new UID, the destructive operation fails with `REASON_LOST_AFTER_RESET` (the reset has one reason
  for all its failures); an awaiting entry moved out of the reset folder is treated the same way as (1); an entry
  moved into it that collides is left waiting and meets (1) at the next lookup. Flag changes are re-pointed as before.
  **Limit:** `MailBackend::find_by_message_id` returns one UID, so a duplicate that exists only on the server and
  has not been synced is invisible to the check. It is only as good as the last sync of the destination (the lookup that
  follows a move at once runs before that folder was synced, so it sees the duplicates the device already knew).
- **Op ids are global.** `EngineHandle::undo(op)` and `operation(op)` take no account: ids come from one counter, and the
  UI only holds ids it received from the call that queued the operation.
- **New storage helpers** (`core/storage/src/`, no migration): `folders::set_validity_only`, `messages::count_in_folder`,
  `messages::uids_in_folder`, `messages::count_other_copies`.
