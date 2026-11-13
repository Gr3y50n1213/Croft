# T36b: Pin and follow-up

Folders: `core/` (protocols, mail-core, storage, engine, ffi), `android/` (the Inbox's Pinned section, the Follow up folder, the actions).

Depends on: T36. Read first: `docs/PLAN.md` ("Gmail-style smarts": pin and follow-up are keywords, so they sync),
`docs/ux/server-contract.md` (the queue and Undo), `docs/ux/message-list.md` (S10), `docs/ux/controls.md` (S21's overflow menu).

## Goal
A person can pin a message so it stays at the top of the Inbox, and mark a message for follow-up so it is easy to find again. Both
are kept on the server as keywords, so every client that reads them (other Croft devices, the Companion) agrees.

## Spec
- **Two fixed keywords, never free text.** Pin is `$Pinned` and follow-up is `$FollowUp` (IMAP keywords are case-blind; JMAP sends
  them lower-case, `$pinned` and `$followup`). The core knows only these two (`Keyword::{Pinned, FollowUp}`): no text from the app or
  a message ever goes into an IMAP command as a keyword. The names are recorded in DECISIONS; they are not checked against the IANA
  keyword registry, which this container cannot reach.
- **Through the queue, like Star.** A new flag change (`FlagChange::Keyword(Keyword, bool)`) and operation (`SetKeyword`), applied
  locally first, sent after 1.5 s like read and star, cancelled by Undo while it waits, re-pointed after a confirmed move.
- **A server that cannot keep keywords.** IMAP: the folder's `PERMANENTFLAGS` (from SELECT) must contain `\*` or the keyword itself;
  otherwise the operation fails as `Permanent` (the app's "Couldn't pin" for a refused change) and the local change is rolled back,
  instead of a pin that the next sync silently drops. Clearing a keyword is always sent. JMAP keeps any keyword.
- **Storage (migration 0010).** `is_pinned` and `follows_up` columns on `message`, kept in step with `keywords_json` on every write
  and filled from it for stored mail; a partial index for each, scoped by account.
- **The core (FFI v17).**
  - `set_pinned(ids, value)` and `set_follow_up(ids, value)`.
  - `list_pinned(account: Option)` for the Inbox's Pinned section (at most 20, newest first); pinned messages are left out of the
    Inbox's date-ordered list so nothing shows twice. Other folders show them in place.
  - `MessageRow` and `ConversationMessage` gain `pinned` and `follow_up`.
  - A "Follow up" virtual folder per account (like Starred), newest first.
- **Android:**
  - S10: a Pinned section above the Inbox list (its own header, collapsed to three rows with "Show all" when longer), and a pin and
    follow-up mark on rows.
  - Pin / Unpin and Follow up / Done in the selection toolbar's overflow menu and S21's overflow menu, each with Undo; a pinned
    row in the Pinned section has its own Unpin button.
  - The Follow up folder in the drawer, after Snoozed.
- **Not in this task:** a follow-up reminder at a time (a device-side wake like snooze) and "remind me if nobody replies"; Pin
  and Follow up as swipe actions and on the web (T43: the swipe rules are shared with the web's TypeScript machine, so both change
  together). In OPEN-ITEMS.

## Done when
- Protocol tests: the keyword STORE on IMAP (and its refusal without `\*` in `PERMANENTFLAGS`), the JMAP keyword patch, against the
  test servers in the integration suite.
- mail-core and storage tests: the operation, Undo, the local effect, the columns kept in step with the keywords, migration 0010,
  per-account scoping, the Pinned list and the Follow up folder.
- FFI tests: set, list, and the rows' marks.
- Android unit tests: the Pinned section with Unpin, the toolbar and S21 items with Undo, the Follow up folder.
- CI green.
