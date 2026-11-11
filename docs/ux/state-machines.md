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
