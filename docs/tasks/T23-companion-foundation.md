# T23: Companion foundation

Crate / folder: `companion/` (a Cargo workspace of its own, AGPL-3.0-or-later)

Depends on: phase 1 (the core crates it will reuse from T26 on). Blocks every other phase 2 task.

## Read first
`docs/PLAN.md` (Companion server, Security section: "Companion storage: envelope encryption", "Companion sign-in"), `docs/tasks/PHASE-2.md`,
`LICENSES.md` (the Companion is AGPL-3.0), `CLAUDE.md` security rules, `test-servers/docker-compose.yml`.

## Goal
A Companion that installs with one command, starts, stores its data safely, and has the security base every later task builds on:
per-user data keys wrapped by a master key that never enters the database. No sign-in and no mail yet.

## Spec
- **Workspace.** `companion/Cargo.toml` with one crate `croft-companion` (library plus the `croft-companion` binary). axum 0.8 on tokio,
  rusqlite (bundled SQLite) for storage, `ring` for encryption. No `unsafe`, `missing_docs` on, the same lints as the core. Licence
  headers AGPL-3.0-or-later.
- **Config.** A TOML file (path from `--config` or `CROFT_COMPANION_CONFIG`), every field overridable by an environment variable:
  listen address, public URL, data directory, master key file. Unknown keys are an error. A missing or unreadable master key is a
  startup error with a plain message, never a fallback.
- **Master key and envelope encryption.** The master key is 32 random bytes in a file the operator keeps outside the database
  (created by `init` with mode 0600; refused if it is readable by others). Each user gets a random 32-byte data key, stored only
  wrapped (AES-256-GCM under the master key, with the user id as associated data). A sealed value (a later task's mail password, token,
  cached mail) is AES-256-GCM under the user's data key with a context string and the user id as associated data, so a value moved to
  another user or another field does not open. Master key rotation: `rotate-master-key` rewraps every data key under a new master
  key in one transaction. Keys are wiped from memory after use (`zeroize`).
- **Storage.** SQLite in the data directory, WAL, foreign keys on, forward-only migrations in `companion/migrations/`: `user` (id,
  name, role admin or user, created, disabled), `user_key` (user id, wrapped data key, key version), `audit_log` (time, user id or
  none, action, outcome; never a secret, an address or a body). Every query on user data is scoped by user id.
- **CLI.** `croft-companion init` (config and master key, idempotent: never overwrites a key), `croft-companion user add --name N
  [--admin]` (creates the user and its data key; prints the user id), `croft-companion user list`, `croft-companion rotate-master-key`,
  `croft-companion serve`.
- **HTTP.** `GET /healthz` (200 when the database answers), `GET /version` (the crate version only). Strict security headers on every
  answer (no-store, nosniff, frame-ancestors none, a restrictive CSP), a request body limit, timeouts, graceful shutdown on SIGTERM.
  Plain HTTP on the listen address; TLS is the reverse proxy's job (documented), and the public URL must be https unless it is
  localhost.
- **Docker.** A multi-stage `companion/Dockerfile` (build on the Rust image, run on a slim Debian or distroless image) as a non-root
  user with `/data` as the volume; `companion/docker-compose.example.yml`. No image is pushed anywhere.
- **CI.** A `companion` job: fmt, clippy, tests, and a Docker build (not pushed).
- **Docs.** `companion/README.md`: install with Docker, the master key and how to back it up (losing it loses every user's data),
  rotation, the reverse proxy.

## Done when
- Unit tests: envelope encryption round trip; a value sealed for one user, or under one context, does not open for another; a wrong or
  rotated-away master key opens nothing; rotation keeps every value readable; the master key file with loose permissions is refused.
- Storage tests: migrations apply to an empty database and are idempotent; the audit log refuses nothing and stores no secret; every
  user-data query is scoped by user id (a test lists them, as `every_table_has_an_account_scope_or_is_global` does for the core).
- HTTP tests (axum in-process): `/healthz` and `/version` answer with the security headers; a body over the limit is refused.
- CLI tests: `init` twice leaves the key unchanged; `user add` then `user list` shows the user with no secret.
- CI builds the Docker image.
- Not checkable here: running the image on a real host behind a real reverse proxy. Reported as unverified.
