# T13: Component gallery

Crate / folder: `android/`

Depends on: T12 (theme and screenshot tooling). Blocks: T14, T15, T17, T18 (they build on these controls).

## Read first
`docs/ux/build-order.md` (step 3), `docs/ux/controls.md` (all of it: every table is a checklist), `docs/ux/haptics.md` (Implementation section and event map), `docs/ux/motion.md` (rules and catalogue), `docs/ux/accessibility.md`, `docs/ux/theme.md` (roundness, density), `design/CLAUDE.md`.

## Goal
A debug-only gallery screen that shows every control in every state, so a missing state is a visible gap. Also the one `Haptics` interface the rest of the app will call.

## Spec
- Controls to build (state tables in `controls.md`): filled, tonal, outlined, text and danger buttons; extended FAB (expanded, collapsed, hidden); Send button with all its conditions; filter, input (recipient), suggestion and account-status chips; text fields (all states, password toggle); switch, radio, checkbox with indeterminate; message row (all states: read, unread, pressed, focused, selected, dragging placeholder, new, starred, encrypted, pending clock, failed edge); attachment chip; banner, snackbar (one at a time, replacement rule), dialog, bottom sheet; menus (at most seven items, destructive last); icon-only button with tooltip.
- Every control: 48 dp minimum touch target, visible 3 dp focus ring with 2 dp gap, disabled at 38 percent / 12 percent and still announced, an `Error` state that is never colour alone. Colours, radii, durations and springs come from the tokens; no literals (add a lint or a test that greps for `Color(0x` and `dp` radii outside the theme package).
- Gallery is reachable only in debug builds (a `debug` source set, not a hidden gesture in release).
- `Haptics` interface with the semantic events listed in `haptics.md` (`ThresholdArmed`, `ThresholdDisarmed`, `Committed`, `Rejected`, `ToggleOn`, `ToggleOff`, `SelectionStart`, `SelectionTick`, `Tick`, `DragStart`, `Success`, `Failure`), a central level filter (Off, Subtle, Full) and a 40 ms same-event guard. The real implementation uses `HapticFeedbackConstantsCompat`; a `RecordingHaptics` fake lives in test fixtures. Switches, long-press and the destructive-dialog confirm call it from the gallery.
- Long-press: system long-press time, 1 dp lift, `Haptics` call, then the result; moving past touch slop first cancels.
- Every animation has the reduced-motion substitute from the catalogue and can be interrupted by touch.

## Done when
- Screenshot tests (tooling from T12) for each component in every state it has, in light and dark, at font scale 1.0 and 2.0, LTR and RTL. A test fails if a state listed in `controls.md` has no gallery entry (keep the state list as data, iterate it).
- At font scale 2.0 no text is clipped and the FAB collapses to an icon (test on bounds).
- Semantics tests: each icon-only control has a content description; each disabled control still appears in the semantics tree as disabled; touch-target size is at least 48 dp for every interactive node.
- `Haptics` tests: level filter drops the right events at Off and Subtle; the 40 ms guard drops a repeat and allows one after 40 ms (fake clock); the recording fake stores order.
- Reduced-motion tests: with Animation level Reduced and Off, each animated control uses its substitute; an interrupt test (touch during animation) hands control to the input.
- Lint/grep test: no hard-coded colour, duration or radius outside the theme package.
- On a device (not checkable here): ripple and state-layer feel, real haptic strength, focus ring with a hardware keyboard, tooltip timing.
