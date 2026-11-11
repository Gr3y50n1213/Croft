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
