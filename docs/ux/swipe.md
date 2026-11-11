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
