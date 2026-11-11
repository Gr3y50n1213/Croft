# Croft — UX, motion and wireframes

Nov 5, 2026

## Interaction principles

Seven rules apply to every screen. When a new screen or feature is unclear, these decide.

1. **Fast first.** Every action shows its result on screen at once and syncs in the background. No spinner ever blocks a tap.
2. **Reversible by default.** Delete moves to Trash and archive is a move, so both can be undone. Only Delete forever, Empty trash and Discard draft ask first, and no setting turns that off. A mistaken read or star change is reversed by the same swipe or tap again.
3. **One gesture, one meaning.** Tap opens, long-press selects or offers more choices, swipe acts on one message. A gesture never means something different on another screen; on message text, links and images, long-press is the system's own menu.
4. **Motion explains.** An animation shows where something went or what changed. It never blocks input, can always be interrupted, and is shortened or removed under reduced motion.
5. **Feedback in layers.** Visual first, haptic second, screen-reader announcement third. A haptic is never the only signal.
6. **Honest status.** Offline, syncing, sending, failed and needs-sign-in are always visible and always say what happens next.
7. **Same logic everywhere.** Android, web and later iOS share state rules and design tokens from the core; only the native look differs.

**Starting values.** Numbers in this spec (thresholds, durations, undo times) are starting points to tune on a real phone, not final.

## Screen inventory

Every screen has an ID used in the wireframes, in issues and in test names (for example `S14 swipe right`). All screens exist on Android and web unless noted; iOS reuses the same list in phase 4. Wide layouts are W01 to W04 and T01.

| ID | Screen | Reached from | Key logic |
| --- | --- | --- | --- |
| S01 | Welcome | First launch | One primary action: Add your email. Secondary: I run a Companion, and Try with sample mail (a read-only demo mailbox made on the device, no account and no network). States that no Croft account is needed; links to the privacy policy and the licences |
| S02 | Add account | S01, drawer, S44 | Continue stays disabled until the address is valid; shortcuts for Gmail, Outlook, iCloud, Other |
| S03 | Finding your server | S02 | Probes run in parallel and the first match wins; each shows waiting, running, found or skipped; failure leads to S05 |
| S04 | Sign in | S03 | Password or app password; OAuth providers open a browser tab, never an embedded view; wrong password shows inline; a self-signed certificate asks once to trust it and pins that exact certificate to this server |
| S05 | Server settings | S03 failure, S45 | Protocol, host, port, security, username; no unencrypted option except for private-network addresses, shown with a red warning; Test connection reports each step; advanced options stay hidden until needed |
| S06 | Connect a Companion | S01, S58 | URL or QR code; TLS check; sign in with passkey or single sign-on; skippable; explains what it unlocks (web, push, search) |
| S07 | Notifications and push | After first account | Explains why before the system permission prompt, then method (UnifiedPush, Google push via the gateway on Play builds, Apple push on iPhone, or periodic sync); Send test; skipping is allowed and the app never needs push to work |
| S08 | Pick your look | Onboarding, S50 | Live preview; mode and palette; applies instantly |
| S09 | First sync | After sign-in | Skeleton rows; newest 50 messages first, older mail in the background; Continue in background always available |
| S10 | Inbox | Launch | Search pill, category chips, unified or per-account list in a container, extended FAB, bottom bar |
| S11 | Selection mode | Long-press, avatar tap | Contextual bar with count, select all and actions; swipe and FAB off; Back exits |
| S12 | Navigation drawer | Menu icon, edge swipe | Mailboxes with unread counts, labels, accounts, Settings |
| S13 | Account switcher | Avatar in search pill | All accounts, each with a status chip; add and manage |
| S14 | Swipe right | Drag a row | Default Mark read or unread; the row lifts as a card and a separate blue pill fills the space it leaves, with the action icon centred in it |
| S15 | Swipe left, armed | Drag a row | Default Delete; red pill; past the threshold the pill goes to full colour, the icon enlarges and a haptic fires |
| S16 | After action with Undo | Release when armed | Row collapses, list closes the gap, snackbar with Undo |
| S17 | Pull to refresh and new mail | Pull down, push arrives | Loading indicator; an n-new pill when scrolled down |
| S18 | Offline and sync problems | Connection or sign-in change | Offline banner, per-account status chips, Needs sign-in state |
| S19 | Inbox zero | Empty folder | Calm empty state with last-synced time; different copy for Trash, Spam and Search |
| S20 | Conversation | Tap a row | Older messages collapsed, newest expanded, action bar on top, reply bar below |
| S21 | Options and blocked content | Overflow menu, banners | Remote images blocked bar, unsubscribe banner, attachment chips, overflow actions |
| S22 | Invitation in a conversation | Message with a calendar invite | RSVP card with Yes, Maybe, No and a conflict warning; sends the reply and adds the event |
| S23 | Compose | FAB, Reply, mailto link | From identity picker, To, Cc and Bcc with autocomplete from contacts and history, a lock on each recipient chip when a key is found, subject, body, toolbar; autosaves to the server Drafts folder |
| S24 | Encryption check | Send, when encryption is required and a recipient has no key | Names who has no key. Send unencrypted or Cancel. Never downgrades silently |
| S25 | Schedule send | Send long-press, compose overflow menu | Presets and custom date and time; needs a Companion if the phone may be off |
| S26 | Undo send and Outbox | After Send | Undo snackbar; Outbox lists Queued, Sending and Failed with Retry, Edit, Discard |
| S27 | Search | Search pill | Recent searches, suggestions, filter chips |
| S28 | Search results | Typing, submit | Highlighted matches, active chips, scope; on-device index first, then server |
| S29 | Snooze | Toolbar, swipe action | Presets and date picker; message leaves the inbox and returns, unread, at wake time (see Snooze logic) |
| S30 | Move to | Toolbar, swipe action | Search, recent folders, folder tree, create new; labels on servers that support them |
| S31 | Unsubscribe | Banner | One-click unsubscribe where the sender supports it, otherwise a prefilled email; always asks first, because it contacts the sender; optional delete of old mail |
| S32 | Notifications | New mail | Grouped per account; up to three chosen actions from Archive, Delete, Mark read, Reply |
| S33 | Home widget | Android home screen | Latest rows and a compose button; tap opens the conversation |
| S34 | Agenda | Bottom bar, Calendar | Day-grouped list, sticky today header, new event FAB |
| S35 | Week | View switch | 3 days on phones; current-time line; tap an empty slot to create; drag to move with 15-minute snap |
| S36 | Month | View switch | Month grid with event dots; selected day agenda below |
| S37 | Event | Tap an event | Time, place, attendees, RSVP buttons for invites, edit and delete |
| S38 | Event editor | FAB, Edit | Title, all-day, times, repeat, reminders, calendar, attendees; Save needs a title; discard asks first |
| S39 | Calendars | Calendar menu | Calendars grouped by account with colour and visibility; subscribe to a calendar link |
| S40 | Contacts | Bottom bar, Contacts | Search, A to Z scroller, account filter, add |
| S41 | Contact | Tap a contact | Mail and call actions, recent conversations, key status, edit |
| S42 | Edit contact | Edit, add | Name, emails, phones, address, notes, key, save-to account; discard asks first |
| S43 | Settings | Avatar menu, drawer | Grouped list of all settings |
| S44 | Accounts | S43 | Status chips; add, reorder, remove with a list of what is deleted locally |
| S45 | Account | S44 | Name, identities and signatures, folder mapping, sync window, server settings, remove |
| S46 | Notification settings | S43 | Master switch, per account, quiet hours, push method health, test, action buttons |
| S47 | Swipe actions | S43 | Live preview row, right and left pickers, sensitivity, reset |
| S48 | Choose swipe action | S47 | Radio list; each action shows its colour and icon; None |
| S49 | Inbox and categories | S43 | Categories, conversation view, preview lines, undo time, undo send time, mark-read delay, remote content |
| S50 | Appearance | S43, S08 | Live preview, mode, pure black, colour source, palettes |
| S51 | Custom colour | S50 | Colour wheel, hex, from image, style, contrast, preview |
| S52 | Layout and density | S50 | Density, roundness, surface style, avatars, unread emphasis, text size, font |
| S53 | Motion and haptics | S50 | Animation level, haptics level, try-it buttons |
| S54 | Encryption keys | S43 | Keys per identity with status; sign and encrypt defaults; publish key |
| S55 | Key detail and import | S54 | Fingerprint, expiry, export public key, back up private key, revoke, generate, import |
| S56 | Encrypted message | Conversation | Lock or shield badge, signature status, trust prompt, decrypt failure message |
| S57 | Security and app lock | S43 | App lock with timeout, hide in recents, block screenshots (Android only), remote content proxy |
| S58 | Companion and devices | S43 | Status, sign-in method, signed-in devices with remote sign-out, push gateway status, unregister this device, delete my Companion data |
| S59 | Data and storage | S43 | Cache per account, sync window, Wi-Fi-only downloads, clear cache, export settings |
| S60 | About and diagnostics | S43 | Version, licences, privacy policy, copy redacted diagnostics, reset app, source link, and a note that Croft is not affiliated with Google, Microsoft or Apple |
| W01 | Web inbox | Web | Sidebar, list, reading pane; toolbar; keyboard hints |
| W02 | Web conversation and compose | Web | Floating compose window docked bottom right, can minimise, up to three drafts |
| W03 | Web calendar | Web | Week grid with mini month and calendar list |
| W04 | Web settings | Web | Category list plus detail; Appearance with live preview |
| T01 | Tablet and foldable | Android tablets | List and detail side by side; rail navigation; posture-aware layout |

## Navigation model

Three top-level destinations on phones, a sidebar on the web, a rail on tablets. Each destination keeps its own back stack and scroll position, so switching never loses your place.

| Surface | Phone | Tablet or foldable | Web |
| --- | --- | --- | --- |
| Top level | Bottom bar: Mail, Calendar, Contacts | Rail with the same three plus Compose | Left sidebar with folders listed |
| Mailboxes and labels | Drawer (S12) from the menu icon or an edge swipe | Permanent drawer beside the list at 840 dp or wider | Always visible, collapsible |
| Account switch | Avatar in the search pill (S13) | Same | Avatar at top right |
| Compose | Extended FAB; shrinks to an icon when scrolling down, grows back when scrolling up | FAB in the rail | Floating window at bottom right |
| Reading | Full-screen conversation (S20) | List and detail side by side (T01) | Reading pane (W01) |
| Settings | Bottom of the drawer, then S43 | Same | Gear icon, then W04 |

### Back and up rules

- **Conversation back** returns to the list at the same scroll offset, with the opened row's highlight fading out. Predictive back shows the list underneath while the conversation shrinks, and a cancelled gesture returns it. Apps that target Android 16 (API 36) get predictive back by default and the old back key callback is no longer called, so every Back rule here is registered with the system back-callback API and nothing else.
- **Selection mode back** exits selection first (S11), then behaves normally.
- **Compose back** with content autosaves to Drafts and shows a Draft saved snackbar. An empty compose just closes. A Discard confirmation appears only after the person taps Discard.
- **Search back** goes from results to the search screen, then to the inbox with the query cleared.
- **Drawer and sheets** close first, always.
- **Onboarding back** goes one step back; on the first screen it leaves the app.
- **Tapping the active bottom-bar item** scrolls the list to the top; tapping it again at the top refreshes.

### Deep links and entry points

- Notifications open the conversation with the inbox as the synthetic back stack (`croft://message/<account>/<id>`).
- `mailto:` links open compose with fields filled in. A `.ics` file opens the event with RSVP buttons.
- The home widget and share sheet ("Send with Croft") enter at compose or the conversation, never at an intermediate screen.

### Transition vocabulary

| Navigation | Transition |
| --- | --- |
| Row to conversation, FAB to compose, search pill to search | Container transform: the source element grows into the destination |
| Bottom bar siblings (Mail, Calendar, Contacts) | Fade through, no direction implied |
| Into and out of settings pages | Shared axis X, forward and back |
| Sheets | Rise with the default spatial spring; scrim fades with an effects spring |
| Dialogs | Scale from 90 percent and fade in |

## State machines

Every stateful thing has a small, named machine in the core or the UI layer, with a test that walks every transition. The tables below are the source for those tests. A state the person can see must say what happens next.

### Account connection

| State | Meaning | What the person sees | Moves on when |
| --- | --- | --- | --- |
| Connecting | Opening the connection or resuming after sleep | Thin progress line under the search pill after 1 s, not before | Connected, or a failure below |
| Idle | Up to date | Nothing. Last synced time in the account switcher | New mail, a refresh or an action |
| Syncing | Fetching changes or older mail | Slim indeterminate bar; list stays usable | Idle, or a failure |
| Offline | No network, or server unreachable | Banner: Offline. Actions still work and queue (S18) | Network returns, then Connecting |
| Needs sign-in | Password changed or OAuth token revoked | Red status chip on the account and a banner with Sign in | Sign-in succeeds |
| Server problem | TLS error, rate limit or server error | Chip with the reason in plain words, and Retry | Retry works, or automatic backoff (5 s, 30 s, 2 min, 10 min, 1 h, then hourly); queued operations wait and do not use up their retries |
| Paused | Battery saver or Wi-Fi-only rule | Grey chip: Paused to save data. Tap to sync now | Rule no longer applies, or the person taps |

### Outgoing message

| State | Meaning | Shown as | Next |
| --- | --- | --- | --- |
| Draft | Being written | Saved on the device after 1 s without typing; copied to the server Drafts folder after 5 s without typing (at most every 20 s while typing goes on) and on pause | Send, Discard, or left as a draft |
| Undo window | Send tapped, 5 s by default | Snackbar: Sending. Undo (S26) | Undo returns to Draft with the cursor where it was; otherwise Queued |
| Scheduled | Chosen send time in the future | Clock chip in Outbox; held on the device, or on the Companion if there is one | Time arrives, then Queued |
| Queued | Waiting for network or turn | Outbox row: Queued | Sending |
| Sending | Uploading to the server | Outbox row with progress ring | Sent or Failed |
| Sent | Accepted by the server | Row leaves the Outbox; a copy is in Sent | End |
| Failed | Rejected, or retries ran out | Outbox row in red with the reason; notification once | Retry, Edit (back to Draft), or Discard with a confirmation |

### Optimistic row action (archive, delete, mark read, move, snooze)

1. **Applied locally** at once: the row changes, the animation plays and the change is written to the local database, so it survives a restart.
2. **Queued** as an operation with a start time. Moves, deletes, spam and snooze wait for the undo window (3, 5 or 10 s). Read, unread and star wait 1.5 s, so a quick change back cancels both; they have no snackbar.
3. **Undo** inside the window cancels the queued operation, so nothing is sent and nothing needs reversing. For read and star, undo is the same swipe or tap again.
4. **Sent** in the background, in order for each message and batched for each account. While an operation is pending, this device's value wins over what the server reports for the fields it changes. A small clock next to the time shows an operation that is waiting for the network.
5. **Confirmed** (the clock goes away) or **Failed**. A network or server hiccup is retried with backoff and the row stays as the person left it. Only a permanent refusal, such as a folder that no longer exists, puts the row back where it was with a snackbar: Couldn't archive. Retry. A sign-in failure pauses the account and shows Needs sign-in; the queue waits and nothing is lost.

The queue is stored in the database, so a killed app or a restart loses nothing. The server commands behind each action, and what happens when a server cannot do one, are in Server interaction contract.

### Selection mode

| State | Entered by | Behaviour | Exit |
| --- | --- | --- | --- |
| Off | Default | Tap opens, swipe acts | n/a |
| On | Long-press a row, or tap its avatar | Bar shows count, select all and actions; tapping a row toggles it; swipe, FAB and bottom bar are disabled | Back, close icon, or count reaching zero |

### Pull to refresh

| State | Trigger | Visual | Haptic |
| --- | --- | --- | --- |
| Pulling | Drag down at the top | Expressive loading indicator grows with the drag | none |
| Armed | Drag past 80 dp | Indicator morphs to its ready shape | Threshold activate |
| Refreshing | Release while armed | Indicator spins until sync finishes | none |
| Done or Error | Sync result | Indicator shrinks away; an error shows a snackbar | Reject on error |

### Search

Idle, then Typing (150 ms debounce), then Local results (instant, from the on-device index), then Server results merged in (a small Searching server line at the end of the list), then Done. Typing again cancels the server request. If the server cannot search (some IMAP servers), the line reads Showing results from this device only.

### Push health

Working (last test passed and a message arrived through it within 24 h), Delayed (arrivals later than 5 min behind sync), Not working (test failed or token expired). Not working falls back to periodic sync and shows a banner in Settings and on S46 with Fix and Send test.

## Message list logic

### What a row shows

Avatar or sender initial, sender (or recipients in Sent), subject, one-line preview, time, and small markers: unread dot and bold, star, attachment clip, lock for encrypted, a count chip for conversations with several messages. Density changes how much of this appears (see the theme picker). Unread is never shown by colour alone: it is bold text plus a dot, and the screen reader says Unread first.

### Order, grouping and threads

- Newest activity first. A conversation moves to the top when a message arrives in it.
- Conversations are built by JMAP thread IDs where the server has them, otherwise by `References` and `In-Reply-To`, with a subject fallback only when headers are missing. Conversation view can be turned off in settings (S49).
- The unified inbox merges accounts by time and tints the avatar stripe with each account's colour. Per-account inboxes are one tap away in the drawer.
- Date headers (Today, Yesterday, This week, month names) are sticky and are also the fast scroller labels.

### Categories and smart sorting

- Chips above the list: All, Primary, Updates, Promotions, Social (turn individual ones off in S49). The classifier runs on the device, from headers (`List-Id`, `List-Unsubscribe`, `Precedence`, sender domain patterns) and a small model. No mail content leaves the phone.
- On Gmail accounts the server's own category labels are used instead. Correcting a message (Move to Primary) teaches the local rules for that sender and offers Always do this.
- A chip with new unread mail shows a count badge. Counts update live and never reorder the chips.

### Selection and bulk actions

- The toolbar shows only actions that apply to everything selected. Mark as read appears if at least one selected message is unread; otherwise Mark as unread appears.
- Select all selects the loaded page and offers Select all N in this folder when more exist on the server.
- Bulk delete or archive of more than 50 messages asks first; fewer runs at once with Undo.

### Optimistic updates and scroll behaviour

- Removing a row collapses its height and closes the gap; rows below slide up. Adding rows (new mail) above the viewport never moves what the person is reading: the list stays anchored and an N new pill appears instead (S17). At the very top, new rows slide in.
- The scroll offset is restored when returning from a conversation, after a rotation or fold, and after a process restart.
- Paging: 50 rows at first, then 100 at a time, loaded when the person is 15 rows from the end. Skeleton rows show only for the first load of a folder, never when refreshing.

### Folder rules that change behaviour

| Folder | Delete does | Archive does | Notes |
| --- | --- | --- | --- |
| Inbox and custom folders | Moves to Trash | Moves to Archive, or removes the Inbox label on label-based servers | Both have Undo |
| Archive | Moves to Trash | Not offered; Move to inbox instead |  |
| Sent | Moves to Trash | Not offered | Row shows recipients |
| Drafts | Discards the draft (Undo) | Not offered | Tap opens compose |
| Spam | Deletes forever after a confirmation | Not offered; Not spam instead | Mail older than 30 days is purged by the server's own rule, not ours |
| Trash | Deletes forever after a confirmation | Not offered; Restore instead | Empty trash is in the overflow menu and asks first |

If an account has no Trash folder, Croft asks once which folder to use or offers to set the deleted flag and expunge, and remembers the choice (S45, folder mapping). With no Archive folder, Croft offers to create one the first time Archive is used and says so; declining hides Archive for that account. With no Junk folder, Mark as spam is hidden and Block sender is offered instead. Every one of these choices is remembered per account and can be changed in S45.

### Empty, loading and error states

- **Empty inbox** (S19): calm illustration slot, the line You're all caught up, last synced time, and a Compose button. Trash and Spam show their own short copy; Search shows No results for the query with a Search server button when only the local index was used.
- **First load**: skeleton rows with a slow shimmer (static when motion is reduced).
- **Error**: a banner above the list with the reason and a Retry or Fix button, never a full-screen error when cached mail exists.

## Server interaction contract

Every action has one local effect and one server effect. The local effect is instant. The server effect is in the table below, and it is run by the core, never by a screen. Croft reads the server's capabilities once per connection (MOVE, UIDPLUS, SPECIAL-USE, IDLE, CONDSTORE), and the screens use them to decide what is enabled.

| Action | IMAP | JMAP | Gmail over IMAP | If it cannot be done |
| --- | --- | --- | --- | --- |
| Mark read or unread | `UID STORE` add or remove `\Seen` | `Email/set` keyword `$seen` | Same | Read-only mailbox: kept on this device only, and the row says Not saved to server |
| Star | `\Flagged` | `$flagged` | Same | Same |
| Archive | `UID MOVE` to the folder with the `\Archive` attribute; without MOVE, `UID COPY`, then `\Deleted` and `UID EXPUNGE` | `Email/set` mailboxIds | Remove the Inbox label (`X-GM-LABELS`); the message stays in All Mail | No Archive folder: create one, or hide Archive (see folder rules) |
| Delete | MOVE to the `\Trash` folder | `Email/set` mailboxIds | MOVE to the `\Trash` folder | No Trash folder: ask once |
| Delete forever | `\Deleted`, then `UID EXPUNGE`. Without UIDPLUS a plain `EXPUNGE` would also remove other messages already marked deleted, so Croft checks that the folder has none, and otherwise moves to Trash and says why | `Email/set` destroy | Move to Trash, then delete from Trash | Always asks first |
| Spam, Not spam | MOVE to the `\Junk` folder and set `$Junk`; Not spam moves back and sets `$NotJunk` | mailboxIds with `$junk` or `$notjunk` | MOVE to the `\Junk` folder | No Junk folder: hidden, and Block sender is offered |
| Move to, label | MOVE, or COPY then delete | mailboxIds | Add or remove `X-GM-LABELS` | The folder is gone: permanent refusal, row restored |
| Send | SMTP submission, then `APPEND` to Sent unless the server files it itself | `EmailSubmission/set`; the server files Sent | SMTP; Gmail files Sent itself | 4xx is retried; 5xx fails with the reason; a sign-in failure pauses the account |
| Save draft | `APPEND` the new version with `\Draft`, then delete the old UID | `Email/set` create, then destroy the old one | Same as IMAP | Offline: saved on the device and copied later |
| Mark all as read | `UID STORE` over the whole folder, in chunks | Batched `Email/set` | Same | Has Undo |
| Unsubscribe | HTTPS POST `List-Unsubscribe=One-Click` (RFC 8058) from the device, no cookies; otherwise `mailto:` through the account's SMTP; otherwise the page in the browser | Same | Same | Always asks first |

### Rules for every operation

- **Order.** Operations on one message run in the order they were made. Different messages run in parallel, up to four at once per account.
- **Identity.** A message is a folder, a UID and the folder's UIDVALIDITY. A move gives it a new UID (the COPYUID or MOVE response), and any queued operation for that message is re-pointed at the new UID before it runs. Without UIDPLUS, Croft finds the new UID by Message-ID. If UIDVALIDITY changes, the folder's local copy is dropped and pending operations are matched again by Message-ID; any that cannot be matched fail with a snackbar.
- **Local and server values.** While an operation is pending, its message keeps this device's value for the fields it changes. When the operation is confirmed or fails, the server's value is shown. All other fields update from the server as usual.
- **Retries.** An account backs off 5 s, 30 s, 2 min, 10 min, 1 h, then hourly. Operations do not use up retries while the account is offline. A transient answer from a connected server is retried up to 8 times, a permanent refusal fails at once, and a sign-in failure pauses the account and its queue.
- **Safe to repeat.** Every command can run twice without harm (a flag set twice, a MOVE that first checks that the source UID still exists), so a lost reply after a crash corrupts nothing.
- **Never delete locally first.** The local copy of a message is removed only when the server confirms that it is gone.

### Snooze logic

IMAP keywords carry no value, so a wake time cannot be stored on the server as a keyword. The schedule lives where something can act on it.

- **Without a Companion:** the message is hidden on this device until the wake time. Other devices still show it in the Inbox, and the confirmation says Snoozed on this phone.
- **With a Companion:** the Companion moves the message to a Croft/Snoozed folder (a label on Gmail), so every device hides it, and moves it back to the Inbox at the wake time before it sends the push.
- **Waking on Android:** an inexact alarm (`setAndAllowWhileIdle`) starts a background job, so a snooze never fires early and may be a few minutes late in Doze. Croft asks for no exact-alarm permission. A boot receiver registers the alarms again.
- **Waking on iPhone:** a local notification is scheduled, and due snoozes are applied when the app opens or refreshes in the background. **On the web:** when the app is open, or when the Companion wakes it.
- **Returning:** the message goes to the top of the Inbox as unread with a Snoozed chip for 24 hours, and a notification says Back in your inbox. Snoozes that fell due while the app was closed are applied before the list first draws.

## Swipe actions

Swiping a row acts on that one message. The person chooses what each direction does, as in Gmail. Defaults are chosen for triage: **swipe right marks read or unread (blue), swipe left deletes (red).** Delete moves to Trash with Undo, so it is safe on a quick flick.

### Available actions

| Action | Colour and icon | What it does | Notes |
| --- | --- | --- | --- |
| Mark read or unread | Blue, open envelope or closed envelope | Toggles; the icon shows what the swipe will do | Card springs back and updates; no snackbar, swipe again to reverse |
| Delete | Wax red, bin | Moves to Trash | In Trash and Spam it deletes forever and asks first |
| Archive | Green, archive box | Moves to Archive | Not offered in Archive, Sent, Drafts, Trash |
| Snooze | Amber, clock | Opens the snooze sheet (S29) | Card parks at 40 percent with the pill showing while the sheet is open; closing the sheet springs it back |
| Move to | Blue, folder | Opens the folder picker (S30) | Same parking behaviour |
| Star or unstar | Amber, star | Toggles | Row stays in place |
| Mark as spam | Wax red, shield | Moves to Spam, offers Block sender in the snackbar |  |
| None | Grey, dash | Swipe does nothing in that direction | The row does not move at all, not even a hint |

Presets in S47: **Triage** (right Mark read, left Delete, the default), **Gmail style** (both Archive), **Inbox zero** (right Archive, left Snooze), **Tidy** (right Mark read, left Archive).

### Gesture mechanics

- **Start.** The row follows the finger once horizontal movement passes the touch slop (8 dp) and the angle is within 30 degrees of horizontal. Otherwise the gesture belongs to the list's vertical scroll, and a gesture that began as a scroll never turns into a swipe.
- **System gestures win.** A drag that starts inside the system back-gesture inset is ignored by the row, so edge swipes still go back or open the drawer.
- **The row becomes a card.** While dragging, the row is a rounded card that follows the finger one to one: corner radius 17 dp at Standard roundness (10 at Tight, 24 at Round), whatever the list style, easing up from the list's own corners over the first 16 dp. It uses the lowest surface container and the page behind it the container tone, so it reads as lifted from the list; in dark mode it is darker than the background, as in Gmail.
- **The pill.** In the space the card leaves, a separate stadium-shaped pill appears in the action's colour. It is the same height as the row, 4 dp from the card and 8 dp from the row's outer edge, so its width is the drag distance minus 12 dp. Its corner radius is half of its smaller side, so it starts as a round capsule and widens into a stadium. The 24 dp icon is centred in the pill and appears once the pill is wide enough to hold it. Below the threshold the pill uses the action's tonal container colour.
- **Threshold.** The action arms at 35 percent of the row width (Low 45, Normal 35, High 25 in S47). Crossing it: the pill goes to the action's full colour, the icon scales to 1.2 with the fast spatial spring, and the arm haptic fires. Crossing back reverses colour and scale and fires the disarm haptic. Each crossing is one event, with a 40 ms guard against jitter.
- **Fling.** A release faster than 1,200 dp/s in the swipe direction commits if the row has moved at least 20 percent, armed or not.
- **Resistance.** Past 85 percent of the width the card follows the finger at one third speed, so it cannot be dragged off screen by accident.
- **Release.** Not armed: the card springs back and the pill shrinks into the edge, both with the default spatial spring. Armed: the action commits.
- **Contrast.** The icon takes the colour for content on that pill (dark on a light pill, light on a dark pill) so it meets 3 to 1 against the pill in every theme and palette. Gmail's pale pill with a white icon does not.

### Commit sequence

| Step | What happens | Time |
| --- | --- | --- |
| 1 | The card slides off the far side with the fast spatial spring while the pill widens to the full row width, 8 dp in from each edge, its icon still centred | about 0.25 s |
| 2 | Pill and slot collapse together: the height closes and the rows below move up | about 0.25 s, overlapping step 1 |
| 3 | Snackbar rises with Undo (5 s, adjustable to 3 or 10 s) | effects spring |
| 4 | Operation queued, not to run before the undo window ends | background |

For actions that do not remove the row (read, star), the card springs back to place, the pill shrinks into the edge and the row's state cross-fades: the dot disappears, bold relaxes, the star pops. No snackbar is shown for read or star, and the toolbar shows nothing either; that keeps rapid triage quiet. A mistaken swipe is reversed by swiping again.

**Undo** returns the row to its old position with the default spatial spring and a brief highlight flash. Because the operation was only queued, undoing within the window cancels it; nothing is sent to the server.

### Swipe state machine

| State | Moves to | When | Side effects |
| --- | --- | --- | --- |
| Idle | Dragging | Horizontal movement past slop and angle test, and an action is set for that direction | Pill appears |
| Dragging | Armed | Distance reaches the threshold | Arm haptic, icon scale up |
| Armed | Dragging | Distance falls back below the threshold | Disarm haptic, icon scale down |
| Dragging or Armed | Settling back | Release below threshold and no fling | Spring to zero |
| Armed or fling | Committing | Release | Slide out, collapse |
| Committing | Done | Animation ends | Snackbar, op queued |
| Armed (Snooze or Move) | Parked | Release | Sheet opens; card held at 40 percent, pill stays |
| Parked | Done or Settling back | Choice made, or sheet dismissed | Collapse, or spring back |
| Any | Settling back | Another pointer touches, selection starts or the row leaves the screen | Spring to zero, no action |

The machine is plain Kotlin (and TypeScript for the web), with no UI types, and is covered by property tests: it never commits below the threshold without a fling, never fires two arm events without a disarm between, and mirrors correctly in right-to-left layouts. Golden tests also check the pill geometry (gap, inset, width, height and radius) for every drag distance.

### Blocked and special cases

- **Action unavailable for this row** (for example Archive in Sent, or a server that cannot snooze): the pill stays neutral grey with a dimmed icon and never reaches the arming point, the haptic is Reject, and the card springs back when released. A snackbar says Not available here, and screen readers hear it.
- **Unified inbox**: each row uses its own account's capabilities and folder mapping.
- **Selection mode, pull to refresh, and fast scrolling** disable swipes. A second finger cancels the swipe. The first touch on a list that is still scrolling only stops the scroll; it never starts a swipe.
- **Row removed by sync during a drag**: the drag ends without action.
- **Delete forever** (Trash, Spam): the card parks at 40 percent, a confirmation dialog opens, and Cancel springs it back.
- **Right to left**: settings are stored as start to end and end to start, so the directions mirror automatically. Labels read Swipe right in left-to-right and Swipe left in right-to-left for the physical direction, with the stored meaning shown in S47's preview.
- **Dark mode and high contrast**: the card and its page use different surface tones in both modes, so the lifted look holds; at High contrast the card also gets a 1 dp outline.

### Customisation screens

- **S47 Swipe actions** has a live preview row that plays a looped demonstration of each direction, a picker for Swipe right and Swipe left, a sensitivity control (Low, Normal, High), and Reset to default. Delete forever always asks first, so there is no switch for it.
- **S48 Choose swipe action** is a radio list; each option shows its colour and icon so the choice is visible before it is made. Choosing the same action for both directions is allowed.
- **Per-account override** is hidden by default and appears under S45 for accounts with unusual servers.

### Accessibility and other inputs

- Every swipe action is also reachable without a swipe: long-press selection toolbar, the conversation's toolbar, and TalkBack custom actions on each row (the actions come from the same settings, so the announcement says Delete, Mark read and so on).
- Switch Access and keyboard users get the same actions through the actions menu and shortcuts.
- With reduced motion on, the card still follows the finger (that is direct manipulation, not animation), and the slide-out becomes a 100 ms fade and the collapse a short fade-and-close, and the threshold is signalled by colour, icon change and haptic only.
- **Web:** trackpad two-finger swipe works the same as touch. On mouse devices, hovering a row shows quick-action icons at its end (Archive, Delete, Mark read, Snooze) taken from the same settings; they use the same Undo toast. Right-click opens the full context menu with the same actions plus Reply, Forward and Move to.

### Deferred idea

Two-stage swipes (a short pull for one action and a long pull for another, as some apps offer) are left out of version 1. The state machine has room for it, but two actions per direction is harder to learn and easier to trigger by mistake.

## Control states

Every interactive control is built and tested in all of the states below. The component gallery (see the build order) renders each control in every state, so a missing state shows up as a gap. State layers follow Material 3: hover 8 percent, focus 10 percent, pressed 10 percent, dragged 16 percent of the content colour over the container.

### States common to all controls

| State | Look | Behaviour |
| --- | --- | --- |
| Enabled | Normal colours | Responds to input |
| Hovered (web, mouse) | 8 percent state layer | Cursor changes only for links and text |
| Focused (keyboard, switch access) | 10 percent layer and a 3 dp focus ring in the primary colour with a 2 dp gap | Ring never clipped; visible on every surface |
| Pressed | 10 percent layer, ripple on Android, shape morphs slightly on expressive buttons | Action fires on release, not on press, so a touch can be cancelled by dragging away |
| Long-pressed | The pressed layer, then a 1 dp lift once the system's long-press time (about 400 ms) has passed | Full-level haptic, then the menu, selection or drag opens; moving past the touch slop before that cancels it |
| Disabled | Content at 38 percent, container at 12 percent | Not tappable, but still announced by screen readers as unavailable and reachable by switch access; when the reason is not obvious, a long-press or hover explains it, and keyboard focus lands on it so the reason can be read |
| Loading | Label replaced by a spinner; width does not change | Not tappable, announced as Busy; after 10 s the button offers Cancel |
| Error | Container outline and text in the error colour, with an icon and a message | Never colour alone |

Touch targets are at least 48 by 48 dp even when the visible control is smaller. Tapping a disabled control that has a reason shows that reason once as a snackbar. Icon-only buttons show a tooltip with their label (and the shortcut on the web) on long-press or hover, and always carry the label for screen readers.

### Buttons

| Button | Used for | Special states |
| --- | --- | --- |
| Filled | The one primary action on a screen | Loading, then a 600 ms check (success), then back to the label |
| Tonal | Secondary actions beside a primary | Same |
| Outlined | Alternatives (I run a Companion server) | Same |
| Text | Low-emphasis actions, dialog buttons | Same |
| Danger (filled or text) | Delete forever, Remove account, Discard | Always needs a confirmation or Undo |
| Extended FAB | Compose | Expanded, collapsed (icon only, on scroll down), hidden (selection mode, keyboard open), pressed, focused |
| Send | Compose toolbar | See below |

### Send button

| Condition | State | Why |
| --- | --- | --- |
| No recipient | Disabled; tap explains Add a recipient | Cannot send |
| A recipient chip has an invalid address | Disabled; the chip shows its error and tap explains Fix the address | Cannot send |
| Recipient but empty subject and body | Enabled; tap asks Send without subject or text? | Matches common expectation |
| Everything present | Enabled |  |
| Attachments over the server's size limit | Disabled; the attachment chip shows a warning with the limit (from the server's SIZE announcement, or 25 MB if it gives none) | The server would refuse it |
| Attachment still uploading | Enabled; message waits in Outbox until ready, with a progress ring | Never blocks the person |
| Offline, or the account needs sign-in | Enabled; the Outbox shows Queued, with a Sign in banner when needed | Nothing is lost; see Outgoing message |
| Tapped | Plays the send animation, then the Undo snackbar | See state machine |
| Scheduled | Icon changes to a clock, label Schedule | Long-press on Send chooses a time; Schedule send is also in the overflow menu |
| Encryption required but a recipient has no key | Enabled with a warning icon; tap opens the S24 encryption check, with Send unencrypted or Cancel | Never silently downgrades |

### Chips

| Chip | States |
| --- | --- |
| Filter chip (category, search filters) | Unselected, selected (tonal fill and a check that slides in), pressed, focused, disabled; with an optional count badge |
| Input chip (recipients) | Normal, selected (for delete), error (invalid address, red outline and icon), with key lock icon when a key is found; tap opens a menu (Move to Cc, Move to Bcc, View key, Remove), long-press lifts it to drag between To, Cc and Bcc |
| Suggestion chip | Normal, pressed |
| Account status chip | Connected (hidden), Syncing, Offline (grey), Needs sign-in (red), Server problem (amber) |

### Text fields

States: empty with label, focused (label floats, 2 dp primary outline), filled, error, disabled, read-only, with leading and trailing icons, with supporting text and counter. Validation runs when the field loses focus, not while typing; after a first error it updates live so it clears as soon as it is fixed. Password fields have a show or hide toggle that is announced. Autofill and password managers work on all sign-in fields.

### Switches, radios, checkboxes

Switch: off, on, pressed (thumb grows), disabled on and off. Toggling applies immediately; a setting that needs a restart says so below the label. Radio rows are fully tappable. Checkboxes support an indeterminate state for Select all.

### Message row

| State | Look |
| --- | --- |
| Read | Regular weight, no dot |
| Unread | Bold sender and subject, dot at the leading edge, time in strong colour |
| Pressed | Ripple; row lifts 1 dp |
| Focused (keyboard) | Focus ring around the row |
| Selected | Primary container tint; avatar turns into a check with a 180 ms flip |
| Dragging (swipe) | The row becomes a rounded card that follows the finger, and a separate pill fills the space it leaves (see swipe actions) |
| New (just arrived) | Brief 1.2 s tint that fades, only if visible on screen |
| Starred | Amber filled star at the trailing edge |
| Encrypted | Lock icon; signed-but-invalid shows a warning triangle instead |
| Action pending offline | A small clock glyph next to the time, no change to layout |
| Action failed | After a permanent refusal the row returns with a red edge and a Retry snackbar |

### Attachment chip

Not downloaded (icon, name, size), downloading (progress ring, tap to cancel), downloaded (tap to open), failed (red, tap to retry), blocked (executables and scripts are flagged and open only after a warning).

### Banners, snackbars, dialogs, sheets

- **Snackbar**: one at a time. A new one replaces the old one. The earlier action stays queued and still runs on schedule; only its Undo button goes away. Deleted mail stays in Trash, archived mail in Archive and a queued send in the Outbox, so nothing is lost when the button goes. Auto-dismisses after the undo window, or 4 s without an action; where the system reports a longer accessibility timeout (Android's recommended timeout, VoiceOver users), the longer one wins. It sits above the bottom bar and moves above the keyboard.
- **Banner**: persistent until fixed or dismissed (Offline, Needs sign-in, Remote images blocked, Unsubscribe). At most one at a time; the most urgent wins.
- **Dialogs**: used only for destructive or irreversible choices. The destructive button is on the right, text style, in the error colour. Back and scrim tap cancel. On iPhone they are system alerts, with the destructive button in the destructive style.
- **Bottom sheets**: drag handle, drag to dismiss, back to dismiss, keyboard-aware. A sheet that holds unsaved input asks before dismissing.

### Menus and long-press

Long-press is a shortcut, never the only way: every result in the table has another route.

- **Timing.** A long-press starts at the system's long-press time (about 400 ms on Android, 0.5 s on iPhone). Moving past the touch slop first cancels it, so it never fights a scroll or a swipe. The pressed element lifts 1 dp and the long-press haptic fires (at Subtle and Full); the result opens at that moment, not on release.
- **Menus.** At most seven items, grouped. Destructive items come last, in the error colour. An item that cannot apply is hidden; if the reason helps, it is shown disabled with the reason. Menus open at the pressed element, mirror in right-to-left, and close on Back or a tap outside. Arrow keys, Enter, Escape and typing a letter work with a hardware keyboard. There are no submenus: a longer choice opens a sheet. A menu action that removes a row gets the same Undo snackbar as its swipe.
- **Same actions everywhere.** A menu offers the actions of the toolbar enablement table below, so the swipe, the toolbar, the menu and the keyboard cannot disagree.
- **Web and iPhone.** On the web, right-click is long-press and opens the same menu, and Shift-click selects a range. On iPhone the menus are system context menus with a preview.

| Target | Long-press does | Tap does | Another route |
| --- | --- | --- | --- |
| Message row | Enters selection mode with that row selected | Opens the conversation | Tap the avatar, or Select in the row's accessibility actions |
| Contact row | Enters selection mode | Opens the contact | Select in the actions menu |
| Send button | Opens Schedule send (S25) | Sends | Schedule send in the compose overflow menu |
| Recipient chip | Lifts it to drag between To, Cc and Bcc | Opens the chip menu: Move to Cc, Move to Bcc, View key, Remove | The chip menu |
| Attachment chip | Menu: Save to device, Share, Open with; Remove in compose | Opens or downloads | The attachment bar's overflow |
| Mailbox in the drawer | Menu: Mark all as read (with Undo), Sync now, Mailbox settings | Opens the mailbox | Mailbox settings in S45 |
| Icon-only button | Tooltip with its label, and the shortcut on the web | Acts | The label is always there for screen readers |
| Link in a message | System menu with the real destination shown first: Open, Copy link address, Share | Opens the link after a destination check | None needed |
| Image in a message | Save image, Copy, Share (once loaded) | Opens the viewer | None needed |
| Message text | The system's text selection | Nothing | None needed |
| Calendar event | Lifts it to move, with 15-minute snaps | Opens the event | The time fields in the editor |
| Empty calendar slot | Creates an event at that time | Nothing | The new event button |
| Search history item | Removes it from the history, with Undo | Runs the search | Clear history in the search menu |

**Conversation overflow menu (S21):** Mark as unread, Move to, Snooze, Mute thread, Print, Block sender, Report spam (Not spam in the Spam folder), Show original. An item the account cannot do is hidden. Mute thread is kept on the device, and as the `$muted` keyword on JMAP servers.

### Toolbar enablement rules (conversation and selection)

| Action | Enabled when | Label changes |
| --- | --- | --- |
| Archive | Not already in Archive, and the account has an archive folder | none |
| Delete | Always | Delete forever in Trash and Spam |
| Mark read or unread | Always | Follows the current state |
| Snooze | Always on phones, because the device wakes the message; on the web only with a Companion | Hidden on the web without a Companion |
| Move to | At least one other folder | none |
| Reply all | More than one other recipient | Hidden otherwise |
| Encrypt toggle | A key exists for the sending identity | Shows the key state |

When offline, actions stay enabled and queue. Only actions that need the server's answer (searching the server, fetching an uncached attachment) are disabled and say why.

## Motion system

Motion in Croft uses springs, not fixed-time curves, so a gesture that is interrupted or reversed carries its velocity into the next movement. This follows the Material 3 Expressive motion scheme: **spatial** springs move or reshape things and may overshoot a little; **effects** springs change colour or opacity and never overshoot.

### Spring tokens

Values are the ones used by the Material 3 Expressive motion scheme (stiffness and damping ratio). The settle time is my estimate from those numbers for a spring that has come within 2 percent of its target; confirm on a real device. SwiftUI values come from response = 2π / √stiffness.

| Token | Stiffness | Damping | Settles in about | SwiftUI response | Use |
| --- | --- | --- | --- | --- | --- |
| Spatial fast | 800 | 0.6 | 0.24 s | 0.22 | Small things: switch thumbs, chips, swipe reveal, star pop |
| Spatial default | 380 | 0.8 | 0.26 s | 0.32 | Most movement: row collapse, sheets, container transforms |
| Spatial slow | 200 | 0.8 | 0.35 s | 0.44 | Full-screen moves and large shape changes |
| Effects fast | 3,800 | 1.0 | 0.07 s | 0.10 | Press feedback, state layers |
| Effects default | 1,600 | 1.0 | 0.10 s | 0.16 | Fades, colour changes, icon cross-fades |
| Effects slow | 800 | 1.0 | 0.14 s | 0.22 | Scrim, large colour shifts such as theme changes |
| Standard spatial fast, default, slow | 1,400, 700, 300 | 0.9 | 0.12, 0.17, 0.26 s | 0.17, 0.24, 0.36 | Used in place of the expressive set when Animation level is Reduced |

In Compose these come from `MaterialTheme(motionScheme = MotionScheme.expressive())`, so component animations pick them up automatically; custom animations ask the scheme for `fastSpatialSpec()`, `defaultSpatialSpec()`, `slowSpatialSpec()` and the three effects specs. The tokens also live in `design/tokens.json`, and the web and iOS layers generate their own code from it (CSS `linear()` easing approximating each spring on the web, `.spring(response:dampingFraction:)` on iOS), so the three apps feel alike.

### Rules

1. Move only `transform` and `opacity` (and colour) during an animation. Never animate layout or size in a way that makes text reflow; collapses clip instead.
2. Every animation is **interruptible**: touching the screen mid-animation hands control to the finger.
3. Nothing waits for an animation to finish before accepting input.
4. The first frame of feedback appears within one frame of the touch.
5. A screen that is not visible runs no animations.
6. Stagger is used rarely, at most 30 ms between items, and never on lists longer than 8 items.

### Animation catalogue

| Animation | Trigger | What moves | Token | Reduced-motion substitute |
| --- | --- | --- | --- | --- |
| Row to conversation | Tap a row | Row expands to the full conversation (container transform) | Spatial default | Cross-fade, 100 ms |
| Predictive back | Back gesture | Conversation shrinks and fades, list shows beneath | Spatial default, follows the finger | Plain fade |
| FAB to compose | Tap FAB | FAB grows into the compose sheet | Spatial default | Cross-fade |
| FAB extend and collapse | Scroll direction | Label slides and fades, FAB narrows | Spatial fast | Instant swap |
| Search pill to search | Tap | Pill expands to full screen | Spatial default | Cross-fade |
| Tab switch | Bottom bar tap | Fade through between destinations; indicator pill morphs | Effects default, spatial fast for pill | Instant swap |
| Settings drill-down | Open or close page | Shared axis X | Spatial default | Cross-fade |
| Sheet open and close | Tap, drag | Rises from the bottom; follows the finger when dragged | Spatial default | Fade |
| Dialog | Open | Scale from 90 percent, fade | Effects default | Fade |
| Menu open and close | Long-press, overflow tap, right-click | Menu scales from 90 percent at the pressed element and fades; closes with a fade | Effects default | Fade |
| Avatar to check | Select | Avatar flips to the check mark | Spatial fast | Cross-fade |
| Star toggle | Tap | Scale 1, 1.35, 1 with colour fill | Spatial fast | Colour change only |
| Long-press lift | Press held past the long-press time | The pressed element lifts 1 dp and grows to 102 percent; the menu or selection opens at that moment | Spatial fast | Highlight only, no scale |
| Unread change | Mark read or unread | Dot fades and shrinks; text weight cross-fades | Effects default | Same |
| Swipe reveal | Drag | Card offset, the pill that fills the gap, icon scale 1.2 at the arm point | Spatial fast | Colour and icon change only |
| Swipe commit | Release armed | Card slides off while the pill widens to full width, then both collapse | Spatial fast, then spatial default | 100 ms fade, short collapse |
| Swipe park or cancel | Release armed on Snooze or Move, or release short of the arm point | Armed: the card slides to 40 percent and holds while the sheet is open, then leaves or returns. Not armed: the card springs back and the pill shrinks into the gap | Spatial default for parking, spatial fast for cancel | Card moves without bounce; pill fades |
| Undo | Tap Undo | Row returns, brief highlight | Spatial default, effects default | Fade in |
| New mail | Arrival | Rows slide down (at top) or N new pill rises (scrolled) | Spatial default | Appear without movement |
| Pull to refresh | Drag | Expressive loading indicator grows, morphs at threshold | Spatial default | Static spinner |
| Send | Tap Send | Compose card shrinks toward the bottom, then the snackbar rises | Spatial default | Fade out |
| Button loading | Tap | Label to spinner, width fixed; success check for 600 ms | Effects default | Same |
| Change refused by the server | A permanent refusal after an optimistic change | The row returns to its place with a red edge that fades after 2 s, and the Retry snackbar rises | Spatial default, effects default | Row reappears; red edge without fade |
| Snackbar | Show and hide | Rises and fades | Spatial fast, effects default | Fade |
| Chip selection | Tap | Fill changes, check slides in | Spatial fast for width, effects for fill | Instant |
| Drag and drop | Drag a recipient chip, event or reorderable item | The item lifts with a shadow, neighbours make room, and it settles into the slot on drop | Spatial fast while dragging, spatial default on drop | Item jumps to the slot; no gap animation |
| Switch | Toggle | Thumb moves and grows, track colour changes | Spatial fast | Instant |
| Theme change | Pick a palette or mode | Whole app cross-fades colour roles | Effects slow | Instant swap |
| Skeleton loading | First load | Slow shimmer, 1.2 s loop | Linear | Static placeholders |
| Onboarding probes | Each probe resolves | Check pops in; row text fades to its result | Spatial fast | Instant |
| Offline banner | Network change | Slides down and pushes the list | Spatial default | Instant |

### Animation level setting

S53 offers **Follow system** (default), **Full**, **Reduced** and **Off**. Follow system reads the OS setting (Android animator duration scale, web `prefers-reduced-motion`, iOS Reduce Motion). Reduced swaps the expressive springs for the standard set, removes overshoot and replaces all travel with short fades. Off makes every change instant.

### Performance budget

Animations hold 60 fps on a mid-range phone and use the display's full rate where available. In the list, the 95th-percentile frame time under a swipe is a release check (see the build order). On the web, only compositor properties animate and long lists are virtualised.

## Haptics

Haptics confirm a moment in a gesture; they never carry information alone and are never used for passive events (mail arriving while the list is open makes no vibration). The rule is one haptic per gesture moment, and the app asks the system's own haptic API, so the person's system touch feedback setting is respected.

### Levels

S53 sets the level. **Off** sends none. **Subtle** keeps only the moments that carry meaning: swipe threshold, long-press, errors and send. **Full** adds ticks, toggles and selection. The default is Subtle on every platform; Full is always the person's choice.

### Event map

| Event | Level | Android | iOS | Web |
| --- | --- | --- | --- | --- |
| Swipe reaches threshold (arm) | Subtle | `GESTURE_THRESHOLD_ACTIVATE` (API 34); `CLOCK_TICK` below | Impact, medium | 10 ms pulse on supporting Android browsers |
| Swipe leaves threshold (disarm) | Full | `GESTURE_THRESHOLD_DEACTIVATE` (API 34); none below | Impact, light | none |
| Swipe commits | Full | `CONFIRM` (API 30); `KEYBOARD_TAP` below | none (arm already fired) | none |
| Action not available | Subtle | `REJECT` (API 30); `LONG_PRESS` below | Notification, error | none |
| Send accepted | Subtle | `CONFIRM` | Notification, success | none |
| Send failed, or sync error | Subtle | `REJECT` | Notification, error | none |
| A change the server refused (row returns) | Subtle | `REJECT` | Notification, error | none |
| Enter selection mode (long-press) | Subtle | `LONG_PRESS` | Impact, medium | none |
| Context menu opens (long-press, right-click) | Subtle | `LONG_PRESS` | Impact, medium | none |
| Add or remove a row in selection | Full | `SEGMENT_TICK` (API 34); `CLOCK_TICK` below | Selection changed | none |
| Star on or off, switch on or off | Full | `TOGGLE_ON` or `TOGGLE_OFF` (API 34); `CONTEXT_CLICK` below | Selection changed | none |
| Pull to refresh reaches threshold | Full | `GESTURE_THRESHOLD_ACTIVATE` | Impact, light | none |
| Fast scroller moves to next date or letter | Full | `SEGMENT_FREQUENT_TICK` (API 34); `CLOCK_TICK` below | Selection changed | none |
| Start dragging an item to reorder | Full | `DRAG_START` (API 34); `LONG_PRESS` below | Impact, medium | none |
| Item dropped after a drag | Full | `GESTURE_END` (API 30) | Impact, light | none |
| Each slot passed while reordering, or each 15 minutes while dragging an event | Full | `SEGMENT_TICK` | Selection changed | none |
| Slider step (text size, density) | Full | `SEGMENT_TICK` | Selection changed | none |
| Undo tapped | Full | `CONTEXT_CLICK` | Impact, light | none |
| Long-press on Send (schedule) | Subtle | `LONG_PRESS` | Impact, medium | none |
| Destructive dialog confirmed | Subtle | `CONFIRM` | Notification, warning | none |

The API 34 constants and their fallbacks come through `HapticFeedbackConstantsCompat` in AndroidX, so the app writes one call and older phones degrade gracefully. On the web the Vibration API exists only in some Android browsers and iOS Safari has no equivalent, so the web setting is off by default and the web app never depends on it.

### Implementation

- One `Haptics` interface per UI layer with semantic events (`ThresholdArmed`, `ThresholdDisarmed`, `Committed`, `Rejected`, `ToggleOn`, `ToggleOff`, `SelectionStart`, `SelectionTick`, `Tick`, `DragStart`, `Success`, `Failure`). Screens call the events; only the platform file knows the constants.
- A central level filter drops events the current level does not include, and a guard drops the same event if it fires again within 40 ms.
- Tests use a recording implementation: a swipe that crosses the threshold, comes back and crosses again must record Armed, Disarmed, Armed, then Committed, in that order.
- New-mail notification vibration belongs to the Android notification channel, which the person controls in system settings; the app does not override it.
- S53 has Try it buttons that fire each event so the person can feel the difference between Subtle and Full.

## Theme picker

The theme picker (S08 in onboarding, S50 in settings) lets the person change the whole look of Croft in a few taps and see the result before leaving the screen. It has four layers, each independent: **mode**, **colour**, **shape and density**, **type**. Motion and haptics sit beside it on S53.

### What the screen contains

1. **Live preview** pinned at the top: a real, non-interactive inbox built from the actual components (search pill, two rows, chip, FAB, snackbar), not a picture. Every change below updates it within a frame.
2. **Mode**: System, Light, Dark, and a Pure black switch for OLED screens (shown only in dark).
3. **Colour source**: a row of choices, each one a small swatch tile.
4. **Palettes**: the Croft presets, shown as tiles with the primary, secondary and surface colours.
5. **Style**: how colourful the scheme is (see variants below).
6. **Contrast**: Standard, Medium, High.
7. Links to **Layout and density** (S52), **Custom colour** (S51) and **Motion and haptics** (S53).

### Colour sources

| Source | How it works | Availability |
| --- | --- | --- |
| Croft palette | One of the presets below | Everywhere |
| Wallpaper | Colours drawn from the phone's wallpaper through Android's dynamic colour | Android 12 and later |
| Custom colour | A seed chosen by wheel, hex value or a photo (S51) | Everywhere |
| Per-account colour | Not a theme; each account has a colour used for its avatar stripe, status chip and the From chip | Everywhere |

The default is the Croft palette Moss, not the wallpaper, so the app looks the same on every phone and in screenshots. Wallpaper colours are one tap away, and the onboarding picker offers them first on Android 12 and later.

### Croft palettes

Each preset is a seed colour plus a style. Names are plain words, not product-y ones. Seeds are starting values, to be checked on screen.

| Preset | Seed | Feel |
| --- | --- | --- |
| Moss (default) | `#4F6F52` | Calm green, paper-white surfaces in light |
| Wax | `#B4472E` | Warm red, the accent of the brand |
| Slate | `#4A6275` | Cool blue-grey, quiet and neutral |
| Heather | `#77607F` | Muted purple, no gradients |
| Ochre | `#A9741B` | Warm amber, high contrast |
| Peat | `#6B5B4B` | Brown-grey, very low saturation |
| Tide | `#2F6F73` | Teal, slightly cooler than Moss |
| Rowan | `#9A3F52` | Deep berry |

### Scheme generation

- Schemes come from the Material Color Utilities (the same HCT colour model Android's dynamic colour uses): a seed produces tonal palettes and then every colour role for light and dark.
- **Styles**: Tonal spot (default, calm), Neutral (nearly grey), Vibrant, Expressive, Fidelity (stays close to the seed), Monochrome.
- **Contrast levels** use the library's contrast setting: Standard, Medium and High. High contrast is also chosen automatically when the system's high-contrast text setting is on, unless the person has picked a level.
- **Pure black** replaces the darkest surface with `#000000` and keeps the raised surface containers a few tone steps lighter so cards and the list container remain visible.
- The same library exists for Kotlin, TypeScript and Swift. A golden test feeds each preset's seed to all three and requires identical colour roles, so the platforms cannot drift.
- A custom seed with almost no colour (very low chroma) is allowed; the library handles it. The picker shows a note when the chosen style makes the scheme look grey.

### Shape and density (S52)

| Setting | Options | Default | Effect |
| --- | --- | --- | --- |
| Density | Comfortable, Default, Compact | Default | Comfortable: about 88 dp rows, avatar 44, two preview lines. Default: about 72 dp, avatar 40, one preview line. Compact: about 56 dp, avatar 32, no preview |
| Roundness | Tight, Standard, Round | Standard | Scales the corner radii of containers, buttons and chips by 0.6, 1 and 1.4, limited to half the component height. The swipe card's corner is 17 dp at Standard, so 10 at Tight and 24 at Round |
| List style | Container, Flat | Container | Container: the list sits in one rounded surface on a tinted background, as in current Gmail. Flat: rows run edge to edge on the plain background |
| Avatars | Letters, Photos, Hidden | Letters | Photos come from contacts and BIMI logos if the person allows it |
| Unread emphasis | Bold and dot, Bold, Dot | Bold and dot | Never colour alone |
| Show previews | Off, 1 line, 2 lines | By density |  |

Row heights are design starting points to adjust on a phone.

### Type

- **Text size**: a slider from 85 to 150 percent with ticks, multiplied with the system font size; layouts must hold at the combined 200 percent without clipping.
- **Interface font**: System (default), Croft Sans (a bundled open-licence sans) or Atkinson Hyperlegible.
- **Reading font** for message bodies: System, Serif, or Atkinson Hyperlegible. Does not change the interface.
- Message bodies from HTML mail are shown as sent with the reading font size applied. In dark mode only simple light-background mails are inverted; complex HTML mail shows on a light card instead of being recoloured badly.

### Behaviour

- **Preview first**: tapping a palette changes the preview and the app together with an effects slow cross-fade. A snackbar offers Undo for 5 s, so a bad choice is one tap from recovery.
- **Revert**: Reset appearance is in the overflow menu and asks first.
- **Safe fallback**: if the stored theme cannot be read, Croft starts with Moss, Standard, System mode.
- **Share and back up**: Export theme saves `croft-theme.json`; Import reads one; Share offers a `croft://theme?...` link and a QR code. With a Companion connected, appearance syncs between the person's devices; without one it stays on the device.
- **Web**: the same tokens become CSS custom properties; `color-scheme` and the browser's theme colour follow the mode; the picker is the same layout in a settings pane (W04).

### Stored shape

```json
{
  "mode": "system",
  "pureBlack": false,
  "source": "preset",
  "preset": "moss",
  "seed": "#4F6F52",
  "style": "tonalSpot",
  "contrast": "standard",
  "density": "default",
  "roundness": "standard",
  "listStyle": "container",
  "avatars": "letters",
  "unreadEmphasis": "boldDot",
  "textScale": 1.0,
  "uiFont": "system",
  "readingFont": "system",
  "animation": "system",
  "haptics": "subtle",
  "accountColors": { "acct-1": "#4F6F52" }
}
```

### Acceptance checks

- For every preset, in light, dark and pure black, at all three contrast levels, text and icon colours against their containers meet 4.5 to 1 (3 to 1 for large text and graphics), and 7 to 1 at High contrast. This is an automated test on the generated roles.
- Screenshot tests of the preview and of S10 for every preset in light and dark.
- Export then import returns an identical file.

## Customisation inventory

Everything the person can change, where, and the default. This is the full settings tree behind S43. Defaults favour a quiet, safe, private app.

| Area | Setting | Options | Default | Screen |
| --- | --- | --- | --- | --- |
| Look | Mode, palette, style, contrast, pure black | See theme picker | System, Moss, Tonal spot, Standard | S50, S51 |
| Look | Density, roundness, list style, avatars, unread emphasis | See theme picker | Default, Standard, Container, Letters, Bold and dot | S52 |
| Look | Text size, interface font, reading font | See theme picker | 100 percent, System | S52 |
| Feel | Animation level | Follow system, Full, Reduced, Off | Follow system | S53 |
| Feel | Haptics level | Off, Subtle, Full | Subtle | S53 |
| Gestures | Swipe right action, swipe left action | Delete, Archive, Mark read or unread, Snooze, Move to, Star, Spam, None | Mark read, Delete | S47, S48 |
| Gestures | Swipe sensitivity | Low, Normal, High | Normal | S47 |
| Inbox | Categories shown | Primary, Updates, Promotions, Social | Primary, Updates, Promotions | S49 |
| Inbox | Conversation view | On, Off | On | S49 |
| Inbox | Preview lines | Off, 1, 2 | By density | S49, S52 |
| Inbox | Undo time | 3, 5, 10 s | 5 s | S49 |
| Inbox | Mark as read when opened | Immediately, After 2 s, Never | After 2 s | S49 |
| Inbox | After deleting or archiving in a conversation | Back to list, Next newer, Next older | Back to list | S49 |
| Inbox | Confirm before | Archiving (off), Sending (off); deleting forever always asks | As shown | S49 |
| Inbox | Default reply action | Reply, Reply all | Reply | S49 |
| Privacy | Remote images | Ask, Always for known senders, Never | Ask | S49, S57 |
| Privacy | Load images through the Companion proxy | On, Off | On when a Companion exists | S57 |
| Privacy | App lock and timeout | Off, biometric or device credential; Immediately, 1, 5 min | Off | S57 |
| Privacy | Hide content in recent apps, block screenshots (Android) | On, Off | Off | S57 |
| Notifications | Per account on or off, sound, vibration | Per channel | On | S46 |
| Notifications | Quiet hours | Off, or a schedule | Off | S46 |
| Notifications | Action buttons | Archive, Delete, Mark read, Reply (choose two) | Archive, Mark read | S46 |
| Notifications | Push method | UnifiedPush, Google push, Periodic sync | By setup | S07, S46 |
| Accounts | Name, signature per identity, folder mapping, colour | Per account | Detected | S45 |
| Accounts | Sync window | 30 days, 6 months, 1 year, All | 6 months on phones, All on web | S45, S59 |
| Encryption | Sign by default, Encrypt when keys exist, Publish public key | Per identity | Off, On, Off | S54 |
| Data | Download attachments on cellular, cache size | Ask, Always, Never; 1 to 5 GB | Ask, 2 GB | S59 |
| Data | Export or import settings | File, link or QR | n/a | S59 |
| Data | Unregister this device, delete my Companion data | Buttons that ask first; Companion accounts only | n/a | S58 |
| About | Privacy policy, open-source licences | Links | n/a | S60 |

Three principles govern new settings: every setting must have a sensible default so nobody has to visit Settings, a setting that changes behaviour (not just looks) must say in one line what it does, and no setting turns off a safeguard against losing mail: deleting forever always asks, and moves and deletes always get Undo.

## Accessibility and input

These are design requirements, not a late pass. Each has a test in the build order.

### Screen readers

- **Row announcement order:** state first, then content. For example: Unread. From Sam Rivera. Lunch on Friday. 2 hours ago. Has attachment. Starred. The preview line is read last and can be turned off in S49.
- **Custom actions** on each row for every configured swipe action plus Select, Reply and Star. TalkBack and VoiceOver users never need a swipe.
- **Announcements** (polite live regions): Archived, Undo available; Sending; Sent; Offline; Back online; N new messages. Errors are assertive.
- Icons that carry meaning have labels. Decorative ones are hidden. Status chips read as Account Sam, Needs sign-in.
- Focus moves to a sensible place after every change: after a row is removed, focus goes to the next row; after a sheet closes, back to what opened it; after Send, to the inbox's first row.

### Size, contrast and colour

- Touch targets are at least 48 dp (44 pt on iOS). Where the visible control is smaller, its hit area is padded.
- Layouts hold at 200 percent combined text size: rows grow and wrap, density cannot force clipping, and the FAB collapses to an icon.
- Text and icons meet 4.5 to 1 (3 to 1 for large text and graphics) in every preset, and the theme test enforces it. High contrast mode meets 7 to 1.
- Colour is never the only signal. Unread is weight plus dot, errors have an icon and words, swipe actions have an icon and a label in S47, status chips have text.
- Respects the system's bold text, display size, colour inversion, and colour correction.

### Motion and haptics

Reduced motion is honoured everywhere (see the motion system), no flashing content is used, and haptics are optional and never required to understand a state.

### Keyboard, switch and pointer

- Every screen is reachable and operable with a hardware keyboard on Android and the web, and with Switch Access.
- Visible focus ring on every control, never removed.
- Web keyboard shortcuts follow Gmail's by default and are customisable, with `?` showing the full list.
- Swipe, long-press and drag always have a button or menu route, so Voice Control, Voice Access, Switch Access, TalkBack, VoiceOver and a keyboard never need a gesture. Every icon-only button carries a label that Voice Control can say by name.

| Keys | Action |
| --- | --- |
| `j` and `k` | Next and previous conversation in the list |
| `Enter` or `o` | Open the selected conversation |
| `u` | Back to the list |
| `x` | Select the conversation |
| `e` | Archive |
| `#` | Delete |
| `Shift` + `i` and `Shift` + `u` | Mark read and mark unread |
| `s` | Star |
| `r`, `a`, `f` | Reply, Reply all, Forward |
| `c` | Compose |
| `/` | Search |
| `z` | Undo the last action |
| `g` then `i`, `g` then `d`, `g` then `s` | Go to Inbox, Drafts, Sent |
| `?` | Show the shortcuts |

On Android with a keyboard, the same letters work in the list, and `Ctrl` + `Enter` sends from compose.

### Language and layout

- Right-to-left layouts mirror everything including swipe directions and progress indicators; icons that imply direction (back, send) mirror, media controls do not.
- Long names, subjects and addresses truncate in the middle of the line only where an address must stay readable; elsewhere they end with an ellipsis and the full text is available on long-press and to screen readers.
- Dates and numbers use the person's locale; plurals use proper rules per language, not an added s.

### Plain language

Error messages say what happened and what to do, in everyday words: Couldn't connect to mail.example.org. Check the address, or try again. Technical detail sits behind a Details link and in the diagnostics copy (S60). No raw error codes on the main surface.

### Testing

Android Accessibility Scanner and Compose semantics tests on every screen, a TalkBack script for the five core flows (add account, read, swipe or action, reply, search), `axe-core` in the web end-to-end tests, screenshot tests at font scale 1.0 and 2.0, and a manual pass with a hardware keyboard before each release.

## Wireframes

One wireframe for every screen in the inventory: 60 phone screens (S01 to S60), four web screens (W01 to W04) and one tablet screen (T01). They are grouped in sheets of three phones so related screens can be compared, with extra sheets for the swipe gesture in detail, for long-press menus and for theme variants.

### How to read them

- The label under each frame is the screen ID from the inventory, then its name and one line of behaviour. Tap or click an image to enlarge it.
- **Green** is the primary action and selected state. **Wax red** is destructive or error. **Amber** is star and snooze. **Blue** is the read state and moving. Grey blocks are content placeholders.
- Small red tags in a frame are behaviour notes, not UI.
- Frames are 300 by 620 dp on phones. Spacing and sizes are close to final, but copy, icons and exact colours come from the design tokens, not from these pictures.
- These are structure and behaviour, not final visuals. The Moss palette is used because it is the default; every screen must also work in the other presets, in dark mode and at 200 percent text size.
- The source is generated by one script, so the sheets can be regenerated when a screen changes. The SVG files go in the repository's `design/wireframes` folder next to the tokens.

### Sheet list

| Sheet | Screens | Group |
| --- | --- | --- |
| A1, A2, A3 | S01 to S09 | Onboarding: welcome, account, server discovery, sign in, server settings, Companion, notifications, look, first sync |
| B1 to B5 | S10 to S19, S29 to S33 | Inbox: list, selection, drawer, accounts, swipes, undo, refresh, offline, empty, snooze, move, unsubscribe, notification, widget |
| SW | S14 and S15 as one continuous drag | Swipe anatomy: one drag in eight stages |
| LP | S20, S23 and S12 | Long-press menus: a link in a message, a recipient chip, a mailbox in the drawer |
| C1 | S20 to S22 | Reading: conversation, options and blocked content, invitation |
| D1, D2 | S23 to S28 | Compose, recipients and keys, schedule, outbox, search and results |
| E1, E2 | S34 to S39 | Calendar: agenda, week, month, event, editor, calendars |
| F1 | S40 to S42 | Contacts |
| G1 to G6 | S43 to S60 | Settings, including swipe actions (S47, S48), appearance (S50 to S53), encryption (S54, S55, S56), security, Companion, data and about |
| TH | S50 variants | Theme variants: eight previews, four light palettes, three dark and pure black |
| W1 to W4 | W01 to W04 | Web: three-pane inbox, conversation with floating compose, calendar, settings |
| T1 | T01 | Tablet: list and detail, rail navigation |

### Onboarding

![Onboarding sheet 1: S01 Welcome with sample mail, S02 Add account, S03 Finding your server](../design/wireframes/01_A1_onboarding_1.png)

![Onboarding sheet 2: S04 Sign in, S05 Server settings, S06 Connect a Companion](../design/wireframes/02_A2_onboarding_2.png)

![Onboarding sheet 3: S07 Notifications and push, S08 Pick your look, S09 First sync](../design/wireframes/03_A3_onboarding_3.png)

### Inbox

![Inbox sheet 1: S10 Inbox, S11 Selection mode, S12 Navigation drawer](../design/wireframes/04_B1_inbox_1.png)

![Inbox sheet 2: S13 Account switcher, S14 Swipe right, S15 Swipe left armed, drawn with a detached row card and a separate pill](../design/wireframes/05_B2_inbox_2.png)

![Inbox sheet 3: S16 After action with Undo, S17 Pull to refresh and new mail, S18 Offline](../design/wireframes/06_B3_inbox_3.png)

![Inbox sheet 4: S19 Inbox zero, S29 Snooze, S30 Move to](../design/wireframes/07_B4_inbox_4.png)

![Inbox sheet 5: S31 Unsubscribe, S32 Notifications, S33 Home widget](../design/wireframes/08_B5_inbox_5.png)

### Swipe anatomy

![Swipe anatomy: one drag in eight stages with a detached card and a separate pill, delete on the left and mark read on the right, with 35 percent as the Normal arming point](../design/wireframes/09_SW_swipe_anatomy.png)

### Long-press menus

![Long-press menus: a link in a message, a recipient chip being dragged between To, Cc and Bcc, and a mailbox in the drawer](../design/wireframes/10_LP_long_press.png)

### Reading

![Reading sheet: S20 Conversation, S21 Options and blocked content, S22 Invitation](../design/wireframes/11_C1_reading.png)

### Compose, outbox and search

![Compose sheet: S23 Compose, S24 Encryption check, S25 Schedule send](../design/wireframes/12_D1_compose.png)

![Outbox and search sheet: S26 Undo send and Outbox, S27 Search, S28 Search results](../design/wireframes/13_D2_outbox_search.png)

### Calendar

![Calendar sheet 1: S34 Agenda, S35 Week, S36 Month](../design/wireframes/14_E1_calendar_1.png)

![Calendar sheet 2: S37 Event, S38 Event editor, S39 Calendars](../design/wireframes/15_E2_calendar_2.png)

### Contacts

![Contacts sheet: S40 Contacts, S41 Contact, S42 Edit contact](../design/wireframes/16_F1_contacts.png)

### Settings

![Settings sheet 1: S43 Settings, S44 Accounts, S45 Account](../design/wireframes/17_G1_settings_1.png)

![Settings sheet 2: S46 Notification settings, S47 Swipe actions, S48 Choose swipe action](../design/wireframes/18_G2_settings_2.png)

![Settings sheet 3: S49 Inbox and categories, S50 Appearance, S51 Custom colour](../design/wireframes/19_G3_settings_3.png)

![Settings sheet 4: S52 Layout and density, S53 Motion and haptics, S54 Encryption keys](../design/wireframes/20_G4_settings_4.png)

![Settings sheet 5: S55 Key details and import, S56 Encrypted message, S57 Security and app lock](../design/wireframes/21_G5_settings_5.png)

![Settings sheet 6: S58 Companion and devices, S59 Data and storage, S60 About](../design/wireframes/22_G6_settings_6.png)

### Theme variants

![Theme variants: the same inbox preview in Moss, Wax, Slate and Heather light, then Moss, Wax and Slate dark, then pure black](../design/wireframes/23_TH_theme_variants.png)

### Web

![Web 1: W01 three-pane inbox with hover actions and keyboard hints](../design/wireframes/24_W1_web_inbox.png)

![Web 2: W02 conversation with a floating compose window](../design/wireframes/25_W2_web_compose.png)

![Web 3: W03 week calendar with mini month and calendar list](../design/wireframes/26_W3_web_calendar.png)

![Web 4: W04 settings with the theme picker](../design/wireframes/27_W4_web_settings.png)

### Tablet

![Tablet: T01 list and detail side by side with a navigation rail](../design/wireframes/28_T1_tablet.png)

## Build order and acceptance checks

This is the order for building the interface with Claude Code. Each step ends with checks that must pass before the next begins, so a mistake in the foundations is found before screens depend on it.

| Step | Build | Done when |
| --- | --- | --- |
| 1 | **Design tokens** in `design/tokens.json`: colour seeds, shapes, spacing, type, motion springs, haptic events, densities. A generator emits Kotlin, CSS variables and Swift | Golden test: all three outputs give identical colour roles for every preset; spring values match the motion table |
| 2 | **Theme engine** and the Appearance screens (S50 to S53) on fake data | Contrast tests pass for every preset, mode and contrast level; export then import returns an identical file; theme change cross-fades without a layout jump |
| 3 | **Component gallery**: a debug screen in the app that shows every control in every state from the control-states tables | Screenshot tests for each component, in light and dark, at font scale 1.0 and 2.0, in left to right and right to left |
| 4 | **Message list** on fake data: rows, density, selection mode, skeletons, empty states, new mail pill | List scroll at 120 or 60 fps on a mid-range phone with 5,000 rows; scroll position survives rotation and process death |
| 5 | **Swipe engine**: the pure state machine first, then the gesture layer and the long-press menus, then the settings screens (S47, S48) | Property tests on the machine; haptic recorder shows Armed, Disarmed, Armed, Committed in order; reduced-motion variant tested; TalkBack custom actions present; every long-press result has another route and every menu offers the toolbar's actions; 95th-percentile frame time during a swipe stays under budget in a benchmark |
| 6 | **Operation queue** with a delay per kind (the undo window for moves, deletes, spam and snooze; 1.5 s coalescing for read and star), per-message ordering, and re-pointing of later operations after a move, wired to the core with a fake server | Undo inside the window sends nothing; a killed app resumes the queue; only a permanent refusal restores the row, a transient error retries with backoff, a sign-in failure pauses the account, and a sync never overwrites a change that is still pending |
| 7 | **Onboarding and account screens** (S01 to S09) against the real discovery code | Discovery of three test servers, one wrong password, one self-signed certificate; every failure shows its plain-language message |
| 8 | **Conversation, compose, outbox, search** (S20 to S28) | Autosave survives a crash; Send with no network queues and later sends; encryption warning never silently downgrades |
| 9 | **Calendar and contacts** (S34 to S42) | Create, move, delete and RSVP to events against a test CalDAV server; contact edit round trips |
| 10 | **Settings, security, keys, Companion** (S43 to S60) | Every setting in the inventory changes behaviour as described; app lock hides content in recents |
| 11 | **Web client** (W01 to W04) from the same tokens and state machines | Keyboard shortcuts table works; axe-core finds no serious issues; hover quick actions match swipe settings |
| 12 | **Tablet layouts** (T01) | List and detail at 600 dp and 840 dp widths; fold posture handled; no orientation or resizability locks, because Android 16 ignores them on large screens |
| 13 | **Store readiness**: privacy policy, Data safety and privacy label answers, account deletion, review access, the iOS privacy manifest and the closed test (see `docs/store/READINESS.md`) | A release build passes the Play pre-launch report and an App Store Connect validation upload; a sample mailbox lets a reviewer reach every screen without an account |

### Checks that apply to every step

- Every screen from the inventory has a screenshot test in 3 sizes (compact, medium, expanded), light and dark, font scale 1.0 and 2.0.
- Every animation in the catalogue has a reduced-motion test and an interrupt test (touch during the animation).
- Every haptic event goes through the `Haptics` interface and appears in the recording test.
- No screen blocks on the network. Airplane mode is a test environment for every flow.
- Accessibility Scanner and semantic tests pass; the TalkBack script for the five core flows is run by hand before each release.

### Files for Claude Code to read first

These are already in the repository: `design/tokens.json`, `design/screens.csv` (the inventory), `design/wireframes/` (PNG and SVG), `docs/ux/swipe.md`, `docs/ux/motion.md` and `docs/ux/haptics.md`. Add a short `CLAUDE.md` in `design/` when step 1 starts that says: use the tokens, never hard-code a colour, duration or radius; every new control gets a gallery entry; every new animation gets a reduced-motion variant; every haptic goes through the interface.
