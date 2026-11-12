# T27: The Companion's JMAP API for clients

Crate / folder: `companion/` (and a small read-only hook in `core/ffi`).

Depends on: T26 (each user's accounts and core). Blocks T30 (the web client reads mail through it) and T34 (phones through the
Companion).

## Read first
RFC 8620 (JMAP core) and RFC 8621 (JMAP mail), `docs/tasks/PHASE-2.md`, `docs/tasks/T26-companion-mail-accounts.md`,
`companion/README.md`, `docs/DECISIONS.md` (T24 to T26 rows), the core's reading and organising API: `core/ffi/src/core.rs`
(`list_folders`, `list_messages`, `mark_seen`, `set_flagged`, `move_to`, `trash`, `delete_forever`, `next_change`),
`core/ffi/src/reading.rs` (`open_conversation`, `load_body`), `core/ffi/src/compose.rs` (`identities`).

## Goal
A signed-in Companion user's apps (the web client of T30, later the phones) read and organise that user's mail through a standard JMAP
API, served from the user's own core (T26), so any JMAP client library works and the core stays the only place mail logic lives.

## Spec
- **Session** (`GET /jmap/session`, bearer token of a full session): capabilities `urn:ietf:params:jmap:core` (with its limits) and
  `urn:ietf:params:jmap:mail`; one JMAP account per mail account of the caller (`accountId` `a<id>`); `apiUrl` `/jmap/api`;
  `downloadUrl`, `uploadUrl` and `eventSourceUrl` present as the RFC requires (downloads, uploads and push come with later tasks and
  answer 404 / not implemented until then); a `state` from the core's change sequence.
- **API** (`POST /jmap/api`): the request object, method calls in order, result references (`#` arguments with a JSON pointer,
  including `/*/`), `createdIds` echoed; errors per RFC 8620 (`unknownCapability`, `notJSON`, `notRequest`, `limit`;
  method-level `unknownMethod`, `invalidArguments`, `accountNotFound`, `invalidResultReference`, `cannotCalculateChanges`).
- **Methods.**
  - `Core/echo`.
  - `Mailbox/get` (folders: name, role from the special use, counts, `myRights`), `Mailbox/changes`.
  - `Email/query` (filter `inMailbox`, `hasKeyword` `$flagged`; sort `receivedAt` descending; `position` and `limit`;
    `collapseThreads` false), `Email/changes`.
  - `Email/get` (id, threadId, mailboxIds, keywords `$seen` `$flagged` `$answered` `$draft`, from, to, cc, subject, receivedAt,
    preview, hasAttachment, and with `fetchTextBodyValues` / `fetchHTMLBodyValues` the core's text and **sanitised** HTML bodies;
    remote content stays blocked as in the core).
  - `Email/set` update (`keywords/$seen`, `keywords/$flagged`, `mailboxIds` to one mailbox = move) and destroy (to Trash; already in
    Trash: deleted for good), each through the core's operation queue as on the phone.
  - `Thread/get` (the conversation's emails, oldest first; a thread is named by its lowest email id).
  - `Identity/get` (the account's sending identities).
  - States are the core's change sequence: `/changes` from the current state answers no changes, from any other
    `cannotCalculateChanges` (the client re-fetches), which the RFC allows.
- **Ids** are strings with a kind letter (`a`, `f`, `m`, `t`, `i` and the number); every object is checked to belong to the
  `accountId` the call names, and every call runs on the caller's own core.
- **Left out of T27:** sending (`EmailSubmission`, `Email/set` create) and drafts, uploads and downloads, push (`EventSource`, T28),
  `Email/query` full-text filters and threading collapse, `Mailbox/set`. Each answers `unknownMethod` or a `forbidden`-style set error
  until its task.
- **Limits:** `maxCallsInRequest` 16, `maxObjectsInGet` 256, `maxSizeRequest` the server's body limit; requests per user limited.

## Done when
- Against the core's IMAP test server through a user's account (T26): the session lists the account; `Mailbox/get` shows the folders
  with roles; `Email/query` then `Email/get` (by result reference) lists the Inbox newest first with keywords and a sanitised body;
  `Email/set` marks read, flags, moves and destroys, and the change reaches the test server; `Thread/get` and `Identity/get` answer.
- `/changes` answers no changes from the current state and `cannotCalculateChanges` otherwise.
- Isolation: user B cannot name user A's `accountId` (`accountNotFound`) nor reach A's mailbox or email ids through B's own account.
- Malformed requests get the RFC's error shapes; no route works without a full session.
- Not checkable here: third-party JMAP client libraries against it (the web client of T30 is the first real client).
