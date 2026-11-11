# T04: Sync engine

Crate / folder: `core/mail-core`

## Goal
Join the pieces: backend, planner, queue and storage.

## Spec
- One tokio task per account. Loop: drain `OpQueue::take_due`, call the backend, `confirm` or `reject`; run `sync_plan::plan` per folder using `OpQueue::pending_uids`; apply the plan to storage; emit change notifications that the UI turns into observable queries (a `tokio::sync::watch` or broadcast of changed folder ids is enough).
- After a MOVE confirms, call `OpQueue::confirm_moved(id, Some((folder, new_uid)))` with the UID from the COPYUID or MOVE response. That re-points later queued ops on that message. If the server gave no new UID (no UIDPLUS), call it with `None`: the message's later ops are held, and the engine finds the new UID by Message-ID, then calls `repoint`. If it cannot be found, call `abandon(account, from, reason)`, which fails those ops with a reason the row can show. Design this first: write the plan in the PR description before coding. See `docs/ux/server-contract.md` for the rules.
- UIDVALIDITY change: drop the folder's local copy, re-match each pending op by Message-ID (`repoint`), and `abandon` any that cannot be matched.
- Delays: moves, deletes, spam and snooze wait the person's Undo window; read, unread and star wait `FLAG_COALESCE_MS` (1.5 s) and show no snackbar. `enqueue` already applies this per kind.
- Retries: when the connection drops, call `set_offline(account, true)`: nothing is tried and no attempts are used. A `Transient` answer from a connected server goes to `reject`, which backs the whole account off (5 s, 30 s, 2 min, 10 min, 1 h, then hourly) and gives up after 8 attempts per op. `Auth` pauses the account; `Permanent` fails the op and restores the row.
- Wake-ups: IDLE when available, else `next_wake_ms()`, else a periodic timer.
- Nothing blocks the UI path: no function the UI calls waits for the network.

## Done when
- Engine test with a fake backend: mark read, kill, restart, the change reaches the backend exactly once.
- Server flag changes never overwrite an in-flight local change (planner pending set).
- Airplane mode (fake backend returning `Transient`) keeps every user action working and retries with backoff.
- Offline test: while `set_offline` is true, every action works locally and no attempt is counted; reconnecting sends them in order.
- Move test: mark read then archive then star the same message while the server returns the new UID late, and the star lands on the moved copy. Repeat with no UIDPLUS (found by Message-ID) and with UIDVALIDITY changing in between.
