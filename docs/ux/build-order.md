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
