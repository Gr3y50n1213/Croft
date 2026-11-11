# T15: Swipe engine and long-press menus

Crate / folder: `android/`

Depends on: T13 (`Haptics`, controls), T14 (the list). Blocks: T16.

## Read first
`docs/ux/build-order.md` (step 5), `docs/ux/swipe.md` (all: mechanics, commit sequence, state machine, special cases, S47 and S48), `docs/ux/haptics.md`, `docs/ux/motion.md` (springs, reduced-motion substitutes), `docs/ux/controls.md` (Menus and long-press, toolbar enablement rules), `docs/ux/accessibility.md` (custom actions), `docs/ux/customisation.md` (Gestures rows), `docs/ux/theme.md` (roundness), `design/wireframes/` for S14 to S16, S47, S48.

## Goal
S14 Swipe right, S15 Swipe left armed, S16 After action with Undo, S47 Swipe actions, S48 Choose swipe action, plus the long-press menus and the TalkBack custom actions. Actions call a `RowActions` interface (implemented by T16); here it is a fake.

## Spec
- **State machine first.** Plain Kotlin, no UI types, in its own source set so it runs as a pure JVM test. States and transitions exactly as in the table in `swipe.md` (Idle, Dragging, Armed, Settling back, Committing, Parked, Done). Inputs are drag distance, velocity, row width, layout direction, threshold setting (Low 45, Normal 35, High 25 percent), and whether an action is set or available. Output is a list of effects (`Haptic(ThresholdArmed)`, `Haptic(Rejected)`, `Commit(action)`, `Park`, `SpringBack`).
- **Gesture layer.** Starts after 8 dp slop and an angle within 30 degrees of horizontal; ignores drags that start in the system back-gesture inset; a gesture that began as a scroll never becomes a swipe; second pointer cancels; disabled in selection mode, during pull to refresh and fast scrolling; fling above 1,200 dp/s commits from 20 percent; resistance beyond 85 percent. The 40 ms arm/disarm guard is in the machine.
- **Visuals.** Card lifted from the row (corner 17 dp at Standard, scaled by roundness), separate pill (4 dp from card, 8 dp from the edge, width = drag distance minus 12 dp, radius half the smaller side), 24 dp icon centred, scale 1.2 at arming, content colour from the action's on-colour for 3 to 1 contrast, 1 dp outline at High contrast. Commit sequence: slide out and widen (about 0.25 s), collapse the slot, snackbar with Undo for the undo window; read and star spring back with no snackbar.
- **Unavailable action:** grey pill, never arms, `Rejected` haptic, "Not available here" snackbar and announcement.
- **Snooze and Move to:** the machine supports Parked (card held at 40 percent while a sheet is open). Their sheets (S29, S30) are built in T16 (Move to) and later phases (Snooze); until a sheet exists the picker in S48 shows the action disabled with "Coming later".
- **S47 and S48:** live looped preview row, presets (Triage default, Gmail style, Inbox zero, Tidy), sensitivity, Reset; radio list with colour and icon; stored start-to-end so RTL mirrors; physical-direction labels.
- **Long-press menus** from the table in `controls.md` that exist so far (message row: selection; send button comes in T18). Menus have at most seven items, the same actions as the toolbar enablement table.
- **Accessibility:** custom actions on each row for every configured swipe action plus Select, Reply and Star, announced from settings.
- All haptics through `Haptics`; all springs from the motion scheme.

## Done when
- Property tests (kotlin property-testing library, implementer picks and records it) on the pure machine: never commits below the threshold without a fling; never two arm events without a disarm between; never commits when the action is unavailable; RTL gives the mirror image of LTR for the same physical drag; every state is reachable and every row of the transition table has a test.
- Haptic recorder test: drag across the threshold, back, across again, release gives exactly Armed, Disarmed, Armed, Committed.
- Golden tests on pill geometry (gap, inset, width, height, radius) for drag distances sampled from 0 to 100 percent, at all three roundness settings.
- Compose gesture tests with `performTouchInput`: slop, angle, back-inset start, second finger, fling, selection mode disables; a removed row mid-drag ends without action.
- Reduced-motion variant: slide-out becomes a 100 ms fade, collapse a short fade-and-close; threshold still signalled by colour, icon and haptic.
- Semantics test: custom actions present for each configured action and follow a settings change; every swipe result has a non-gesture route (toolbar or menu) in the test.
- Screenshot tests for S14, S15, S16, S47, S48: three widths, light and dark, font scale 1.0 and 2.0, LTR and RTL.
- On a device (not checkable here): feel of the spring, real haptic at the threshold, 95th-percentile frame time during a swipe (keep a macrobenchmark ready), interaction with the system back gesture. Reported as unverified.
