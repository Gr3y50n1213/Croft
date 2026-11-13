# T31: The web inbox (W01), conversation and compose (W02)

Folders: `web/`; `companion/` for sending (T31b).

Depends on: T27 (the JMAP API), T30 (the web app, sign-in, the swipe machine). Blocks T32 (settings, PWA).

Built in two pull requests: **T31a** W01 (reading and organising) and **T31b** W02 (the conversation and compose), which first
needs sending in the Companion's JMAP API (drafts and `EmailSubmission`, which T27 left out).

## Read first
`docs/UX.md` (W01, W02, "Keyboard, switch and pointer", the toolbar enablement table, snackbars), `docs/ux/message-list.md`,
`docs/ux/swipe.md` (the web: hover quick actions from the same settings, trackpad swipes), `docs/ux/server-contract.md` (Undo:
an action is queued in the core and Undo cancels it), `design/wireframes/24_W1_web_inbox.png` and `25_W2_web_compose.png`,
`companion/README.md` ("JMAP for the apps").

## Goal
A signed-in person reads and organises their mail in the browser as on the phone: the same actions, the same Undo, the same
swipe settings, and Gmail-like keyboard shortcuts.

## Spec (T31a, W01)
- **A small JMAP client** (`web/src/lib/jmap/`): the session, method calls with result references, typed `Mailbox/get`,
  `Email/query`, `Email/get`, `Email/set`, `Thread/get`; errors in plain words; the state strings kept for the next call.
- **The sidebar**: the accounts (one JMAP account each) and their folders by role (Inbox, Starred, Sent, Drafts, Outbox, Archive,
  Spam, Trash) with unread counts, then other folders; Compose (opens T31b's window; until then disabled with its reason).
- **The list**: newest first, 50 at a time with more on scroll; sender, subject, preview, time, unread and starred marks;
  selection with checkboxes and `x`; hover quick actions taken from the swipe settings (the defaults of `android/swipe` until W04
  has the setting, T32); a trackpad two-finger swipe drives the same machine (T30b).
- **The reading pane**: the selected email's sanitised HTML (from the Companion: remote content already blocked) in a sandboxed
  frame with no script and no network, or its text; sender, recipients, date, attachments listed (downloads come later); opening
  an email marks it read after the core's delay.
- **Actions** (toolbar, hover, keyboard, the same set everywhere): archive, delete, mark read or unread, star, move to (a folder
  picker), spam. Each goes through `Email/set` (the core queues it); removals and moves show a snackbar with Undo for the undo
  window, and Undo sends the reverse change while the core still holds the operation (the core cancels it, docs/ux/server-
  contract.md).
- **Keyboard**: the table of `docs/UX.md` (`j`/`k`, `Enter`/`o`, `u`, `x`, `e`, `#`, `Shift`+`i`/`u`, `s`, `/`, `z`, `g` then `i`,
  `d`, `s`, `?`); never while typing in a field; `?` shows the list; the hint bar of W01 along the bottom.
- **Fresh**: the list and folders reload on push (T28's Web Push in the browser is T32's) or on focus and every minute meanwhile;
  `/changes` answers lead to a refetch.
- **Accessibility**: landmarks, the list as a listbox with the selection announced, every icon button labelled with its shortcut
  in the tooltip, focus visible, `axe-core` in the end-to-end tests.

## Spec (T31b, W02)
- **The Companion's JMAP gains sending**, as a Croft extension (`urn:croft:params:jmap:compose`; see DECISIONS): drafts
  (`CroftDraft/get`, `/set`, `/check`), reply fields by the core's rules (`CroftDraft/reply`), and the Outbox (`CroftOutbox/send`
  with its undo window, `/undo`, `/get`). Reviewed independently.
- **The conversation**: the thread's emails, newest expanded; Reply, Reply all, Forward.
- **The compose window**: floating bottom right, minimise, up to three drafts docked; To, Cc, Bcc with address chips, subject,
  a plain-text body (rich text later); saved as a draft as on the phone; Send with the Undo snackbar.

## Done when
- T31a: unit tests for the JMAP client and the list's state; Playwright against a real Companion with a mail account on the core's
  in-process IMAP test server (a small test-only binary): the folders and the Inbox show, an email opens in the reading pane, the
  keyboard moves and acts, archive and Undo, star, mark read; no CSP violation; `axe-core` finds no violation on W01.
- T31b: Companion tests for drafts and sending (through the core's test SMTP server); Playwright: reply and send, Undo a send, a
  minimised and a second draft.
- Not checkable here: real mail servers through the web app, Safari and Firefox.
