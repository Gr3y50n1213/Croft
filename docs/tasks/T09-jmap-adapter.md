# T09: JMAP adapter

Crate / folder: `core/protocols` (plus `test-servers/`)

Depends on: T03 (the trait and the IMAP adapter it must behave like), T05 (discovery returns a JMAP session URL), T06 (the sending abstraction). Blocks: nothing in the UI; T10 only needs it to exist.

## Read first
`CLAUDE.md`, `docs/PLAN.md` (protocol table, stack table), `docs/ux/server-contract.md` (the JMAP column), `core/protocols/src/backend.rs`, `docs/tasks/T03-imap-adapter.md`, `docs/tasks/T05-account-discovery.md`, `test-servers/docker-compose.yml`.

## Goal
`MailBackend` for JMAP (RFC 8620 core, RFC 8621 mail), so a Stalwart or Fastmail account syncs and acts like an IMAP one. Nothing outside `core/protocols` may learn which protocol an account uses.

## Spec
- Use the `jmap-client` crate (Stalwart). If it does not fit (no rustls, blocks the planner's snapshot shape, or lacks `Email/changes` handling) write a small client over `reqwest` + `serde_json` instead. Record the choice and the reason in `docs/DECISIONS.md`.
- Session: fetch `/.well-known/jmap`, read capabilities and the account id. TLS rules are the same as T03 (certificate validation always on; pinned fingerprint only when the caller passes one).
- Mapping to the trait: `list_folders` from `Mailbox/get` (roles `inbox`, `archive`, `trash`, `junk`, `sent`, `drafts` become the same special-use values as IMAP); `snapshot` and `fetch_headers` from `Email/query` + `Email/get` (headers, `threadId`, `receivedAt`, keywords, `mailboxIds`); incremental sync from `Email/changes` with the state string stored per account (the planner treats it like a modseq). If the server answers `cannotCalculateChanges`, fall back to a full snapshot.
- Actions as in the contract's JMAP column: flags are `$seen`/`$flagged`/`$junk`/`$notjunk` keyword patches; `move_to` patches `mailboxIds`; `delete_forever` is `Email/set` destroy. A JMAP message id is stable across moves, so `MoveOutcome` reports the same id (no re-pointing needed) and `find_by_message_id` uses `Email/query` with the `header` filter.
- Idempotent: a repeated `Email/set` for the same change must not error; a `notFound` for a destroy counts as success.
- Send: implement `EmailSubmission/set` behind the same sending abstraction T06 defines (a trait or enum chosen there). If T06 made that abstraction SMTP-only, make the smallest change to it and note it in the PR. The server files Sent, so no APPEND.
- Errors: network and timeouts to `Transient`; HTTP 401/403 to `Auth`; `rateLimit` and 5xx to `Transient`; `forbidden`, `notFound` on a mailbox, `invalidProperties` to `Permanent`. No tokens, addresses or message content in error strings.
- Discovery hook: the JMAP result from T05 (`/.well-known/jmap` found) builds this backend; add the constructor and one test.
- Add Stalwart to `test-servers/docker-compose.yml` with a **pinned image version** (not `latest`) and a setup script or config that creates two users. Run the integration tests behind the existing cargo feature `integration`.

## Done when
- `cd core && cargo test` passes without network (parsing and mapping unit tests use recorded JSON fixtures, including `cannotCalculateChanges`).
- With the container up, `cargo test --features integration` passes against Stalwart: list folders (roles mapped), snapshot, fetch headers, mark read, star, move to Trash, delete forever, and an `Email/changes` round trip where a second client changes a flag and the planner sees it.
- Wrong password returns `BackendError::Auth` and never `Transient`; a stopped container returns `Transient`.
- The same behaviour test (a shared test function over `&dyn MailBackend`) runs against the IMAP and JMAP adapters.
- Send through `EmailSubmission` delivers to a second Stalwart user (test reads it back).
- `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings` are clean.
