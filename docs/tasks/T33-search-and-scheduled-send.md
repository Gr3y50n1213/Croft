# T33: Search and scheduled send through the Companion

Folders: `core/` (scheduling in the Outbox), `companion/` (JMAP methods), `web/` (W01's search, scheduling in W02).

Depends on: T27 (the JMAP API), T31b (the compose extension), T19 (the core's search).

Built in two pull requests: **T33a** scheduled send and **T33b** search.

## Read first
`docs/UX.md` (S25 Schedule send, S27 and S28 Search, W01, W02), `docs/ux/state-machines.md` ("Outgoing message"),
`core/ffi/src/search.rs`, `core/ffi/src/compose.rs` (`send`, `undo_send`, `list_outbox`), `companion/src/jmap_compose.rs`.

## A decision first: the core's index, not tantivy
`docs/PLAN.md` names tantivy for the Companion's search. Each user's mail lives in their own SQLCipher database, sealed with their
data key (T26); a tantivy index is a separate set of files that would hold the same text unencrypted on disk. The core already
keeps a full-text index (SQLite FTS5) of headers and downloaded bodies inside that encrypted database, and asks the mail server for
the rest (T19). The Companion uses the core's search: one index, encrypted at rest, the same query language and results as the
phone. Recorded in DECISIONS.

## Spec (T33a, scheduled send)
- **The core**: `Core::schedule_send(account, draft, send_at_ms)`: the message waits in the Outbox until `send_at_ms`, then goes as
  any other (the same retries, Sent copy and draft clean-up). A time inside the Undo window is an ordinary send; more than a year
  ahead is refused (`ComposeError::BadSendTime`). Until it starts going out, `undo_send` takes it back (the draft reopens). The
  Outbox lists it as Queued with its time. FFI v13.
- **The Companion**: `CroftOutbox/send` takes an optional `sendAt` (UTC date); `CroftOutbox/get` gives each message's `sendAt`. The
  sending budget counts a scheduled message when it is scheduled. Every user's core is open while the Companion runs (T26), so
  scheduled mail goes out with no device online, and after a restart.
- **The web app (W02)**: next to Send, "Schedule": tomorrow morning, tomorrow afternoon, Monday morning, or a date and time; the
  snackbar says when, with Undo. A "Scheduled" view in W01 lists what waits, each with Cancel (reopens the draft).

## Spec (T33b, search)
- **The Companion**: `CroftSearch/query` (capability `urn:croft:params:jmap:search`): an account and the query in the core's
  language (`from:`, `to:`, `subject:`, `in:`, `has:attachment`, `is:unread`, `before:`, `after:`, words and phrases), on the stored
  mail first; with `server: true`, the mail server too. Answers the email ids, best first, the matched parts of each subject and a
  body snippet, and whether bodies were missing, the server could not search, was offline or failed. Bounded (at most 100 hits;
  server searches limited per user). Queries are never logged or stored.
- **The web app (W01)**: the search box in the bar (`/` focuses it), results in the list with the matches marked, "Search the server
  too" when the stored mail was not enough, Escape back to the folder.

## Done when
- T33a: core tests (a scheduled message waits, goes at its time, survives a restart, can be taken back before, is refused a year
  out); Companion tests (`sendAt` through JMAP, the budget, isolation); Playwright: schedule, see it in Scheduled, cancel it.
- T33b: Companion tests (results through JMAP, isolation between users and accounts, bounds, the capability); Playwright: search
  finds a message by a word in its subject and body, highlights it, and opens it; axe-core clean on the results.
- An independent review of the new JMAP methods.
