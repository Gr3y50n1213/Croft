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
