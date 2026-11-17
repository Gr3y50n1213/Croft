# T41c: OpenPGP in compose and reading

Folders: `core/` (storage, reader, ffi). The screens are T42.

Depends on: T41a (`croft-crypto`), T41b (keys, the `KeyVault`, `key_status`). Read first: `CLAUDE.md`'s security rules
(encryption never downgrades silently; secrets only in the platform keystore), T41a's and T41b's task files and
`docs/DECISIONS.md` rows, RFC 3156, and `core/ffi/src/compose.rs` (`build`, `send_draft`) and `core/ffi/src/reading.rs`
(`load_body`, `save_attachment`). Security-sensitive: an independent review before merging.

## Goal
A person can send a message encrypted and signed to people Croft has keys for, and is never sent plain text when they
asked for encryption. Encrypted mail they receive opens on the phone without its plain text ever being written to the
database, and a signature is shown for what it is.

## Spec
- **Storage (migration 0016).** `draft.encrypt` and `draft.sign` (0 or 1, default 0). Nothing else: decrypted text is never
  stored.
- **Compose.**
  - `DraftContent` gains `encrypt` and `sign`. `encryption_check(account, draft)` returns, for every To, Cc and Bcc
    address, whether Croft has a usable key (from `key_status`), and whether the account has an own key that can sign and
    encrypt (its secret in the vault).
  - Sending with `encrypt` set: every recipient must have a usable key and the account an own key; otherwise the send is
    refused with `ComposeError::EncryptionNotPossible` naming the addresses without a key. The message is never sent
    unencrypted in its place (the app asks; turning `encrypt` off is the person's choice).
  - With Bcc and `encrypt`: refused (`EncryptionWithBcc`) for now: one message encrypted to everyone would show the Bcc
    recipients' key IDs to the others.
  - The inner entity is the message built as today (body, attachments) without the outer headers; the outer message is
    `multipart/encrypted` (encrypted to the recipients and to the sender, signed when `sign` is set). `sign` alone gives
    `multipart/signed`.
  - The subject stays in the clear for now (protected headers later; noted in OPEN-ITEMS).
  - The server's draft copy of a draft with `encrypt` set is encrypted to the sender's own key only; without an own key it
    is not uploaded (kept on the device).
- **Reading.**
  - `load_body` of a `multipart/encrypted` message: decrypt with the account's own secret keys (from the vault, wiped after
    use), check an inner signature against the keys filed under the `From` address, render the inner entity with the
    existing reader (remote content rules unchanged), and return it with `encryption: Encrypted` and the signature status.
    Nothing decrypted is stored: no text, no snippet, no search index; it is decrypted again at each open.
  - `multipart/signed`: verified against the keys filed under the `From` address; the body as today.
  - `MessageBody` gains `encryption` (`None`, `Encrypted`, `CannotDecrypt` with a reason: no own key, no vault, not for
    this key, damaged) and `signature` (`Unsigned`, `Valid { fingerprint, signer_address }`, `NotVerified { issuer }`).
    "Signed by" is stated for the signer only, never "encrypted by" (a signed message can be re-encrypted by anyone).
  - Attachments inside an encrypted message are listed and saved through the same calls, decrypting again.
  - Autocrypt gossip is not read (later).
- **The core (FFI v24).**

## Done when
- storage tests: the new draft columns round-trip; migration 0016 on a filled database.
- FFI tests: send encrypted and signed to a recipient with a key, decrypted by the recipient's key fixture with a valid
  signature; refused (and nothing queued) when one recipient has no key, when there is no own key, and with Bcc; a signed
  message verifies; reading an encrypted message gives the text and the status, and the database file holds none of its
  text afterwards; a message for another key gives `CannotDecrypt`; an attachment inside an encrypted message is saved;
  the server draft copy of an encrypted draft is encrypted.
- An independent review, with its findings fixed.
- CI green.
