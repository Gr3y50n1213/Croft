# Croft patches to async-imap 0.12.0

This folder is the source of `async-imap` 0.12.0 as published on crates.io (checksum
`f97b87216c9f0ccc63c516263169085fa34bba25633f6d96c6a9b9ca1fd5f70a`), with the changes below. It is
used through `[patch.crates-io]` in `core/Cargo.toml`. It is dual licensed MIT OR Apache-2.0
(`LICENSE-MIT`, `LICENSE-APACHE`), copyright the async-imap authors; those files are unchanged.
It is not a workspace member (`exclude = ["vendor"]`), so Croft's `cargo fmt` and `cargo clippy`
do not touch it.

Every change in the source is marked `CROFT-PATCH`. Files not listed here are byte-for-byte the
published ones. Not copied from the published package: `.cargo-ok`, `.cargo_vcs_info.json`,
`.cirrus.yml`, `.github/`, `.gitignore`, `Cargo.lock`, `Cargo.toml.orig`.

## Why

A security review found that the stock crate writes every encoded command (including
`LOGIN "user" "password"`) and every decoded server response (mail content) to the `log` crate at
`trace` level. Croft capped the level at build time (`log/max_level_debug`), but any crate in a
build can lift that cap again with `log/release_max_level_*`, and then the password reaches
whatever logger the app installs. The reliable fix is to have no such calls at all. The review
also found that the crate sized its read buffer from a literal length the server announces, up to
512 MiB, before any data arrived.

## Changes

### 1. Logging calls removed (`src/imap_stream.rs`)

The crate has no other `log::` / `trace!` / `debug!` / `info!` / `warn!` / `error!` calls (checked
with a search over all of `src/`). These four were the only ones, and all are removed:

| Where | Removed call | What it logged |
| --- | --- | --- |
| `ImapStream::encode` | `log::trace!("encode: input: {:?}, {:?}", msg.0, from_utf8(&msg.1))` | the tag and the whole command line, so `LOGIN "user" "password"`, `APPEND` data, search terms, folder names |
| `ImapStream::decode` | `log::trace!("decode: input: {:?}", from_utf8(buf))` | the whole buffered server response: message headers, flags, folder names |
| `ImapStream::decode` | `log::trace!("decode: incomplete data, need minimum {min} bytes")` | a number derived from the response (a literal length chosen by the server) |
| `ImapStream::decode` | `log::trace!("decode: incomplete data, need unknown number of bytes")` | nothing secret; removed so the function has no logging at all |

`Cargo.toml`: the `log` dependency is removed. The crate cannot log any more, and a logging call
added back by a future re-vendor does not compile. `croft-protocols` also has a test that scans
this folder for logging calls (`tests/vendored_async_imap.rs`).

### 2. Parse errors no longer carry the response (`src/imap_stream.rs`)

`ImapStream::decode` built its `io::Error` for an unparsable response from
`format!("{:?} during parsing of {:?}", other, String::from_utf8_lossy(buf))`: the parser error
(which holds slices of the input) and the whole response text. Errors travel to logs and the UI.
The error is now the fixed text "the server sent a response that could not be parsed".

### 3. Buffer limit lowered, and size arithmetic cannot overflow (`src/imap_stream.rs`)

- `Buffer::MAX_CAPACITY` 512 MiB -> **64 MiB**. `grow` already returned
  `io::Error("incoming data too large")` above the limit; with the new value a `{500000000}`
  literal is refused at once, before anything is allocated, as an `io::Error` (not a panic).
  The existing `test_buffer_grow` test uses the constant, so it still holds.
- `grow` uses `checked_add`, and the `decode_needs` update uses `saturating_add`, so a huge announced
  size is an error rather than an arithmetic overflow (a panic in debug builds and a wrong size in
  release builds on 32-bit targets such as armv7 Android).

### 4. Hooks for running the sign-in exchange outside the crate (`src/client.rs`)

The stock `Client::login` and `Client::authenticate` skip an untagged `* BYE` and turn a closed
connection into `ConnectionLost`, so a caller cannot tell "the server hung up on the password"
from a network failure. Added to `impl Client`:

- `pub async fn run_command(&mut self, command) -> Result<RequestId>` (sends one tagged command),
- `pub async fn run_command_untagged(&mut self, command) -> Result<()>`,
- `pub fn into_session(self) -> Session<T>` (the caller has seen the server accept the sign-in).

`croft-protocols` (`src/imap/conn.rs`, `sign_in`) uses them to read the answer itself.

### 5. Build settings (`Cargo.toml`)

- `[lints.rust] deprecated = "allow"`: the crate uses the deprecated `unsafe_pinned!` macro of
  `futures-util` and compiles with warnings; as a path dependency those would show in our build
  output. This is upstream's code, not ours. (This crate, being third party, contains `unsafe`
  through that macro; Croft's own crates still `forbid(unsafe_code)`.)

## Updating this copy

Re-vendor the new release from crates.io, re-apply the five changes above, and check that
`grep -rnE 'log::|trace!|debug!|info!|warn!|error!' src` finds nothing. Prefer sending changes 3
and 4 upstream. Then `cargo test -p croft-protocols` and `cargo test --release -p croft-protocols`.
