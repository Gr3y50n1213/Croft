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
