# Phase 3: Smart inbox, calendar and contacts, OpenPGP (task plan)

Roadmap phase 3: "Categories, snooze, one-tap unsubscribe, on-device search index. Calendar and contacts on Android and web
(CalDAV, CardDAV). OpenPGP sign and encrypt, WKD and Autocrypt key discovery." Gate 4 after it is 1.0, after an external security
review. Build-order steps 9 and 10 (`docs/ux/build-order.md`). The on-device search index is already done (T19, SQLite FTS5).

Each task gets its own file (Goal, Spec, Done when) before it is built, as the master prompt asks. The order follows the
dependencies; the details are in the task files as they are written.

| Task | Scope | Depends on |
|---|---|---|
| T35 | Inbox categories: the classifier in the core (headers `List-Id`, `List-Unsubscribe`, `Precedence`, sender patterns, and rules learnt from corrections; Gmail's own category labels on Gmail accounts), the category chips in S10 with unread counts, Move to Primary teaching the sender's rule, S49 Inbox and categories | phase 2 |
| T36 | Snooze (T36; pin and follow-up as T36b): snooze as a device schedule in the core and the queue (S29 presets and picker; Android wakes with an inexact alarm and a boot receiver; with a Companion, the Croft/Snoozed folder); pin and follow-up as IMAP or JMAP keywords that sync | T35 |
| T37 | One-tap unsubscribe (S31): RFC 8058 `List-Unsubscribe-Post` from the core, otherwise a prefilled email; always asks first | T35 |
| T38 | Contacts, in two parts. T38a: CardDAV sync in the core (discovery, a small client over the core's own HTTPS client), vCard 3 and 4 that keeps unknown properties, contact storage per account, recent recipients, autocomplete. T38b: Android S40 to S42 and compose autocomplete. JMAP Contacts later | phase 2 |
| T39 | Calendar: CalDAV (and JMAP Calendars where the server has it) sync in the core, iCalendar with recurrence and time zones, reminders; Android S34 to S39 | T38 (the shared DAV client) |
| T40 | Invitations: iMIP in mail as an RSVP card in the conversation; RSVP from S37 | T39 |
| T41 | OpenPGP in the core: keys generated or imported on the device (private keys in the keystore, never in SQLite), sign, encrypt, decrypt, verify; WKD and Autocrypt key discovery; encryption never downgrades silently | phase 2 |
| T42 | Encryption on Android: S54 and S55 (keys, import, export, backup, revoke), S56 (the lock badge and signature status), the compose lock and its "recipient has no key" question | T41 |
| T43 | Web: categories, snooze, pin and unsubscribe in W01, contact autocomplete in W02 | T35 to T38 |
| T44 | The rest of build-order step 10: S57 Security and app lock (hide in recents, block screenshots), S59 Data and storage, and the settings in the S43 inventory not built yet | phase 2 |

Test servers: CalDAV and CardDAV tests need a server with both; the choice (Stalwart's own, or Radicale beside it) is made in
T38's task file. Nothing in this phase sends mail content off the device: the classifier, snooze schedule and keys stay local.
