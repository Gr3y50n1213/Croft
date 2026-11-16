# T41a: OpenPGP in the core: the crypto crate

Folders: `core/crypto/` (new workspace member `croft-crypto`, over rPGP). Storage, the keystore callback, the FFI, key discovery over
the network and the compose and reading paths are T41b; the Android screens are T42.

Depends on: phase 2. Read first: `docs/PLAN.md` ("Encryption (PGP)", the stack row for OpenPGP), `CLAUDE.md`'s security rules
(secrets only in the platform keystore; encryption never downgrades silently), RFC 9580 (OpenPGP), RFC 3156 (PGP/MIME),
Autocrypt Level 1, and draft-koch-openpgp-webkey-service (WKD). Security-sensitive: an independent review before merging.

## Goal
One small, pure, well-tested crate that does every OpenPGP operation Croft needs, so the rest of the core never touches packets:
make and read keys, sign, encrypt, decrypt and verify mail in the PGP/MIME format, read and write Autocrypt headers, and compute
WKD addresses. It never stores, logs or prints a secret; secret key bytes are wiped when dropped.

## Spec
- **Keys.**
  - `generate(user_id, now)`: a v4 key that every common client reads: an Ed25519 primary key (certify and sign) and a
    Curve25519 ECDH encryption subkey, a self-signature with Croft's preferences (AES-256 and AES-128, SHA-512 and SHA-256, no
    compression), no expiry. Returns the secret key (binary, in a wiping buffer) and its public certificate.
  - `read_public(bytes)` and `read_secret(bytes, passphrase)`: armored or binary; the bindings are checked; a protected secret
    key is unlocked with the passphrase and returned unprotected (the keystore protects it from then on). A file with more
    than one key, a key without a usable encryption or signing key, an unreadable or oversized file (over 1 MiB) are refused
    with a fixed error.
  - `info(cert, now)`: fingerprint, key ID, the user IDs' addresses, created, expires, revoked, can encrypt, can sign,
    the primary algorithm. Expired, revoked or SHA-1-bound components are never used.
  - `export_public(cert)` (armored) and `export_secret(secret, passphrase)` (armored, protected with the passphrase, for a
    backup).
- **Messages (RFC 3156).**
  - `encrypt(inner_mime, recipients, signer)`: one OpenPGP message, signed by `signer` when given, encrypted (SEIPDv1,
    AES-256) to every recipient's valid encryption key and to the sender's own key (so the Sent copy can be read). It
    returns the whole `multipart/encrypted` body. A recipient without a usable encryption key is an error naming which.
  - `sign(inner_mime, signer)`: a `multipart/signed` body with a detached signature over the canonical inner part.
  - `decrypt(body, secret_keys, certs)`: the inner MIME and a signature status (signed by a known key and valid; signed but
    the key is unknown; invalid; not signed); the session key never leaves the crate.
  - `verify(signed_part, signature, certs)`: the same status for `multipart/signed`.
  - Bounds: at most 50 MiB in (the reader's own message cap) and the same out; at most one level of compression; a message
    that is neither literal data nor one encrypted and/or signed message is refused.
- **Autocrypt (Level 1).**
  - `parse_header(value, from)`: `addr`, `prefer-encrypt` (`mutual` or nothing) and `keydata`; refused when `addr` is not the
    `From` address, when an unknown critical attribute is present, or when the key is unusable; at most 64 KiB of key data.
  - `header(addr, cert, prefer_mutual)`: the minimal key (primary, the user ID for `addr`, the encryption subkey) in base64,
    folded.
- **WKD.** `wkd_urls(address)`: the advanced and direct addresses (z-base-32 of SHA-1 of the lower-cased local part, with
  `?l=`), only for a syntactically valid address; the fetch itself is T41b's (public addresses only).
- **Never** a secret in `Debug`, an error or a log; errors are fixed categories.

## Done when
- Unit tests: generate, export and read back (public and passphrase-protected secret); a wrong passphrase; an expired and a
  revoked key not used; encrypt to two recipients and decrypt with each; sign-then-encrypt verified as good, a tampered body
  as invalid, an unknown signer as unknown; `multipart/signed` verified over canonical line ends; Autocrypt header round trip
  and the refusals; WKD URLs against the specification's example (`Joe.Doe@Example.ORG`).
- Interoperability: messages and keys from GnuPG (fixtures made with made-up keys) decrypt and verify.
- clippy and fmt clean; the licences of the new crates are recorded (`docs/DECISIONS.md`).
- An independent review, with its findings fixed.
- CI green.
