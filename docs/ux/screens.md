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
