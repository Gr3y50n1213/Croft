# T19: Search

Crate / folder: `core/storage`, `core/protocols`, `core/ffi`, `android/`

Depends on: T18 (stored bodies), T10, T16. Part of the same build-order step 8 as T18, split to keep sessions small.

## Read first
`docs/ux/screens.md` (S27, S28 rows), `docs/ux/state-machines.md` (Search state machine), `docs/ux/message-list.md` (empty states), `docs/ux/controls.md` (filter chips, suggestion chips, history item long-press), `docs/ux/accessibility.md`, `core/storage/migrations/0001_init.sql` (the `message_fts` table and its triggers), `docs/ux/server-contract.md`.

## Goal
S27 Search and S28 Search results: the on-device index first, instantly, then the server merged in.

## Spec
- **Local index:** query `message_fts` (FTS5) scoped by `account_id` through the existing join on `message.id`. Parse a small query language: words, `"phrases"`, `from:`, `to:`, `subject:`, `is:unread`, `is:starred`, `has:attachment`, `in:folder`, `before:` and `after:`. Escape input so no query can raise an FTS syntax error. Highlight offsets come from `snippet()` or `highlight()`. Note that the body column only fills when a body is stored (T18), so results show a note "Searched headers and downloaded mail" until the person taps Search server.
- **Server fallback:** add `search(folder or all, query) -> Vec<RemoteHeader>` to `MailBackend`: IMAP `UID SEARCH` (with `OR`, `TEXT`, `FROM` mapped; use `ESEARCH` if advertised), JMAP `Email/query` with a filter. A server that cannot search answers with a typed "not supported" and the screen says so in plain words. Server hits are merged by message identity into the local list.
- **State machine:** Idle, Typing (150 ms debounce), Local results, Server results merged (a small "Searching server" line at the end), Done; typing again cancels the server request; a failed server search shows a banner, never loses local results.
- **S27:** recent searches (stored in the settings table per account scope, removable with Undo, Clear history in the menu; no cloud), suggestion and filter chips. **S28:** highlighted matches, active chips, scope (all accounts or one), empty copy "No results for the query" with a Search server button when only local ran. Back goes from results to S27 to the inbox with the query cleared.
- Search results use the T14 row component; actions from results go through T16.
- Privacy: queries are never logged and are sent only to the account's own server. The demo mailbox searches locally only.

## Done when
- Rust unit tests: query parser (every operator, quotes, hostile input such as `"`, `*`, `NEAR(`, `)`) yields a safe FTS string or a typed error, never a panic or SQL error.
- Rust tests on a populated in-memory database: ranking order is stable; `account_id` scoping (a message from another account never appears); an update and a delete are reflected by the triggers; body text appears only after the body is stored.
- Integration (container servers, feature `integration`): server search on Dovecot or GreenMail and Stalwart returns the expected UIDs for FROM, SUBJECT and body text; unsupported search maps to the typed error (use the in-process server with SEARCH disabled).
- JVM tests (host library): typing "lunch" shows local results within the same frame budget of a test dispatcher, then merges server hits; typing again cancels the previous server request (assert on the fake backend); airplane mode shows local results and the "cannot reach server" line.
- Compose tests: history add, remove with Undo, clear; chip filters change the query; Back sequence.
- Screenshot tests for S27 and S28 (with highlights, empty, offline variants): three widths, light and dark, font scale 1.0 and 2.0, LTR and RTL.
- On a device (not checkable here): perceived search speed on a mailbox of tens of thousands of messages, keyboard behaviour, highlight legibility on a real screen.
