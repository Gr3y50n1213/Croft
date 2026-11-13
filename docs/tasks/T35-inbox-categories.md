# T35: Inbox categories

Folders: `core/` (protocols, mail-core, storage, engine, ffi), `android/` (S10 chips, S49).

Depends on: phase 2. Read first: `docs/ux/message-list.md` ("Categories and smart sorting"), `docs/ux/customisation.md` (the
Inbox row), `docs/ux/screens.md` (S10, S49), `docs/PLAN.md` ("Gmail-style smarts").

## Goal
The Inbox can be split into Primary, Updates, Promotions and Social, decided on the phone from headers and from the person's
own corrections. No mail content leaves the device, and nothing is guessed from the body.

## Spec
- **Headers.** Sync also fetches `List-Id`, `List-Unsubscribe`, `List-Unsubscribe-Post`, `Precedence` and `Auto-Submitted`
  (IMAP `HEADER.FIELDS`, JMAP `header:...:asText`). They are stored on the message; T37 uses the unsubscribe ones.
- **The classifier** (`core/mail-core/src/category.rs`, pure) takes the sender and those headers, plus the account's sender rules,
  and gives one category, in this order:
  1. a rule the person made for this address, then for its domain;
  2. **Social**: a sender at a known social network's notification domain (a short, fixed list);
  3. **Updates**: `Auto-Submitted` other than `no`, a mailing list (`List-Id`), or a sender whose local part is an automated one
     (`noreply`, `no-reply`, `notifications`, `alerts` and the like);
  4. **Promotions**: bulk mail (`Precedence: bulk` or `junk`) or mail with `List-Unsubscribe` and no `List-Id`;
  5. otherwise **Primary**.
  It is computed when a message is first stored. A sync never changes it, so the person's correction stays.
- **Corrections.** "Move to Primary" (or another category) on a message sets that message's category. "Always do this" stores a
  rule for the sender's address (in a new table with `account_id`) and re-files that sender's existing Inbox mail.
- **The list.** The FFI gives the Inbox filtered by category, and the unread count per category.
- **Android:**
  - S10 shows the chips All, Primary, Updates, Promotions and Social above the Inbox list; categories turned off in S49 are
    hidden, and the default shown set is Primary, Updates and Promotions.
  - A chip with unread mail has a count badge; counts update live and chips never reorder.
  - Rows get "Move to" with a category in the long-press menu.
  - S49 has the categories switch and a toggle per category.
- **Gmail.** Gmail's own categories are not readable over IMAP without a search per category; Gmail accounts use the same local
  classifier (DECISIONS).

## Done when
- Unit tests for the classifier: each rule, the order, and rules beating the built-in lists.
- Storage tests: a migration with the headers and the rule table (scoped by account), a stored category never overwritten by
  a sync, and re-filing by a rule.
- Protocol tests: the IMAP fetch and the JMAP properties carry the new headers, parsed and decoded (folded lines, case).
- FFI tests: the filtered list and the unread counts.
- Android unit tests: the chips' state (shown set, counts, selection) and the S49 settings.
- CI green.
