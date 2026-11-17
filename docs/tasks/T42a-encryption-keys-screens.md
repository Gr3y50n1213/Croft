# T42a: Encryption keys on Android: the key vault, S54 and S55

Folders: `android/` (secrets, mail, ui). The encrypted-message banner (S56) and the compose lock with S24's check are T42b.

Depends on: T41b and T41c (the core's OpenPGP keys and mail, FFI v24). Read first: `CLAUDE.md`'s security rules, the
T41b and T41c task files and `docs/DECISIONS.md` rows, `android/app/src/main/kotlin/com/example/croft/secrets/` (how the
app keeps passwords: one Keystore-wrapped file per entry in `noBackupFilesDir`), the S54 and S55 wireframes
(`design/wireframes/20_G4_settings_4.png`, `21_G5_settings_5.png`). Security-sensitive (the vault): an independent review
before merging.

## Goal
The person can make or import an OpenPGP key for each account on the phone, see it, export its public part, back up the
private part under a passphrase, and remove it. The private part lives only in the Android Keystore-backed store.

## Spec
- **The vault.** `SecretStore` gains per-key entries: `acct-<account>.pgp-<FINGERPRINT>.v1`, sealed by the same
  `SecretBox` and bound to the account and fingerprint (a file copied onto another entry does not open). The account
  sweep, account removal and "Erase all Croft data" cover them. `SecretStoreKeyVault` implements the core's `KeyVault`
  callback on it (store, read, delete), wipes the bytes it was handed, never logs, and reports a busy Keystore as
  `ProviderException.NotReady`. The core gets it when it opens (`setKeyVault`).
- **S54 Encryption keys** (Settings): one card per account: its address, the key's status (Active, Expired, Revoked,
  No key), the short fingerprint and expiry; two defaults for new messages, Sign messages and Encrypt when keys exist
  (kept on the phone with the app's preferences; T42b's compose uses them); Generate and Import when there is no key; tap
  a key for S55. A note that private keys stay on this phone.
- **S55 Key details and import:** the fingerprint in groups of four, the identity, created and expires. Export public
  key (a `.asc` file through the system's save dialog). Back up private key: a passphrase typed twice, then a `.asc`
  file through the save dialog. Remove key (wax red): asks first and says what changes (mail encrypted to it cannot be
  read on this phone without a backup). Import: choose a file or paste; a protected key asks for its passphrase.
- **Not in T42a:** publishing the public key (WKD needs the mail server's help), revocation (the core cannot make a
  revocation yet), the list of other people's keys. Noted in OPEN-ITEMS.

## Done when
- unit tests: the vault's entries per account and fingerprint, bound to their entry, removed with the account and by an
  erase, wiped after use; the core's callback through `SecretStoreKeyVault`.
- screen tests (Robolectric): S54 with a key, without one, the defaults; S55's details, export, backup with mismatched and
  matching passphrases, remove with its question, import of a file and of pasted text, a protected key's passphrase.
- An independent review of the vault, with its findings fixed.
- CI green.
