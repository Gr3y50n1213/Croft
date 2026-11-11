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
