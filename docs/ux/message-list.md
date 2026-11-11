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
