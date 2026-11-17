# T42b: Encryption on Android: the compose lock, S24 and S56

Folders: `android/` (mail/compose, mail/reading, ui/compose, ui/reading). The keys themselves (vault, S54, S55) are T42a.

Depends on: T41c (the core's encrypted and signed mail, FFI v24) and T42a (the vault, `AccountPrefs`' defaults). Read first:
`CLAUDE.md`'s security rules (encryption never downgrades silently), the T41c and T42a task files and `docs/DECISIONS.md` rows,
the S23, S24 and S56 wireframes (`design/wireframes/12_D1_compose.png`, `21_G5_settings_5.png`), and the existing S24 seam
(`mail/compose/EncryptionPolicy.kt`, `ui/compose/EncryptionCheckSheet.kt`).

## Goal
The person can send a message encrypted and signed from the phone, sees before sending who can be encrypted to, and is never
sent unencrypted without choosing it. Encrypted and signed mail they receive says so plainly, and says why when it cannot be
opened.

## Spec
- **Compose (S23).**
  - A lock in the toolbar turns encryption on or off for this message, and "Sign" in the menu does the same for signing.
  - The draft keeps both (`DraftContent.encrypt`, `sign`).
  - A new message starts with the account's defaults (S54): signed when "Sign messages" is on. It starts encrypted when
    "Encrypt when keys exist" is on and every recipient has a key, re-checked as recipients change until the person
    touches the lock.
  - Each recipient chip shows a lock when Croft has a key for that address (`key_status`).
- **Sending (S24).**
  - With encryption on, Send asks the core (`encryption_check`). When a recipient has no key, the sheet names them with
    three choices: "Send without encryption" (turns encryption off for this message, saves, sends), "Remove" (takes
    those recipients out; the person sends again), and Cancel.
  - With no own key, the sheet says so and offers S54.
  - With Bcc, it says an encrypted message cannot have Bcc.
  - The core's own refusals (`NoRecipientKey`, `NoOwnKey`, `EncryptionWithBcc`) open the same sheet. Nothing is ever
    sent unencrypted without that choice.
- **Reading (S56).** Above the body:
  - "Encrypted and signed" or "Encrypted" (a shield) with the signer when valid.
  - "Signed by ..." for a valid signature. "Signed with a key from their mail, not confirmed" (amber) for
    `Unconfirmed`. "Signature not verified" (amber) for `NotVerified`.
  - "Could not decrypt this message" (wax red) with the reason and its fix (no key on this phone: import it in
    Encryption keys; not for this key; damaged; too large).
  - The subject and addresses are never presented as signed.
- **FFI calls used:** `key_status`, `encryption_check`, `MessageBody.encryption` and `signature`.

## Done when
- unit and screen tests: the lock and Sign persist in the draft; the defaults; chips with and without a key; S24 for a missing
  key (send without encryption turns it off and sends; remove takes the recipients out; cancel sends nothing), for no own key
  and for Bcc; the core's refusal opens the sheet; S56's states.
- CI green.
