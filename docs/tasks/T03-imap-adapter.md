# T03: IMAP adapter

Crate / folder: `core/protocols`

## Goal
Implement `MailBackend` for IMAP.

## Spec
- `async-imap` with `tokio` and `tokio-rustls` (webpki roots plus the platform store). TLS 1.2+ only; certificate validation always on. A self-signed certificate is accepted only through an explicit pinned-fingerprint trust record passed in by the caller.
- `list_folders` (SPECIAL-USE attributes first, then `SpecialUse::from_name`), `folder_status`, `snapshot` (UID FETCH 1:* FLAGS; use CONDSTORE / QRESYNC when advertised), `fetch_headers` (ENVELOPE, References, size, internal date), `set_flag`, `move_to` (UID MOVE; fall back to COPY + STORE \Deleted + UID EXPUNGE), `delete_forever`.
- Error mapping: network and timeouts to `Transient`; LOGIN / AUTHENTICATE failures to `Auth`; tagged NO responses to `Permanent`. No passwords or message content in error strings.
- A small in-process IMAP test server in `core/test-support` (a new crate) for fast tests; the container servers in `test-servers/` for the integration suite behind a cargo feature `integration`.

## Done when
- Against GreenMail and Dovecot in containers: list folders, snapshot, fetch headers, set \Seen, move to Trash, delete forever.
- UIDVALIDITY change is detected by `folder_status` and exercised in a test.
- Wrong password returns `BackendError::Auth` and never `Transient`.
