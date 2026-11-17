# T41b: OpenPGP keys in the core: storage, the keystore, discovery

Folders: `core/` (storage, protocols, ffi). Encrypting and signing in compose and decrypting and verifying when reading are
T41c; the screens are T42.

Depends on: T41a (`croft-crypto`). Read first: `CLAUDE.md`'s security rules (secrets only in the platform keystore, never in
SQLite), T41a's task file and `docs/DECISIONS.md` rows, `core/ffi/src/credentials.rs` (how the app already hands the core its
secrets), Autocrypt Level 1, and T37's public-addresses-only fetch (`core/protocols/src/oauth/fetch.rs`). Security-sensitive:
an independent review before merging.

## Goal
Each account can have its own OpenPGP key, made on the phone or imported, with the secret part only ever in the platform
keystore. Croft learns other people's keys from Autocrypt headers in mail they send and from their domain's WKD, keeps them
per account, and says for any address whether Croft can encrypt to it. Every message Croft sends carries the account's
Autocrypt header.

## Spec
- **Storage (migration 0015).**
  - `pgp_key`: per account, a public certificate (binary), its fingerprint, addresses, created, expires, revoked, whether
    it is the account's own (its secret is in the keystore), and where it came from (made here, imported, Autocrypt,
    WKD). One row per fingerprint per account. Never a secret.
  - `autocrypt_peer`: per account and address, the fingerprint last seen in that sender's Autocrypt header,
    `prefer-encrypt=mutual`, the date of that message and the date of the newest message from them.
- **The keystore (FFI callback `KeyVault`).** The app implements `store`, `load` and `delete` of a secret key by account and
  fingerprint (Android: the Keystore, T42). Secret key bytes cross the boundary only into and out of the vault, wiped on
  the Rust side after use; without a vault, nothing secret can be made or imported.
- **Own keys.** `generate_key(account)` (the account's name and address as the user ID), `import_key(account, bytes,
  passphrase)` (a secret key goes to the vault and becomes the account's own; a public key becomes a contact's key),
  `export_public_key`, `export_secret_key(passphrase)` (a backup, protected with the passphrase), `delete_key`. One own key
  per account in use (the newest valid one).
- **Other people's keys.**
  - Autocrypt: when a message is read, its sender's `Autocrypt:` header (checked by T41a against the `From` address) is
    taken if the message is newer than the last one taken from them; messages from the account's own addresses are skipped.
  - WKD: `lookup_key(account, address)` fetches the advanced address, then the direct one: `https` to public addresses only
    (as T37), no redirect, at most 256 KiB, bounded time; a found key must name the address.
  - Import of a public key file.
- **For compose.** `key_status(account, addresses)`: for each address, whether Croft can encrypt to it now (a usable key
  that names the address), from where, and whether the sender asked for encryption (`mutual`).
- **Outgoing.** Every message Croft sends from an account with an own key carries `Autocrypt: addr=...; keydata=...` (the
  minimal key; `prefer-encrypt=mutual` when the account's setting says so).
- **The core (FFI v23).**

## Done when
- storage tests: keys and peers per account, one row per fingerprint, nothing secret in the database (checked by reading the
  file).
- protocols tests: the WKD fetch's request, no redirect, the size cap, a private address refused.
- FFI tests: generate, export and import back through a vault fake; import of a protected key; a public key import; Autocrypt
  taken from a read message (newer only, own addresses skipped, a forged `From` refused); the outgoing Autocrypt header;
  `key_status`; WKD against an in-process server; no vault, no secret.
- An independent review, with its findings fixed.
- CI green.
