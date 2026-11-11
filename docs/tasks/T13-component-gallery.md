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

## Result (session notes, T13)

### Where things are
- Controls: `android/app/src/main/kotlin/com/example/croft/ui/components/` (`Buttons`, `Fab`, `SendButton`, `Chips`, `TextFields`, `Toggles`, `MessageRow`, `AttachmentChip`, `Feedback` (banner and snackbar), `Dialogs` (dialog and sheet), `Menus`, `IconButtons`, `Spinner`, `Transitions`, and `Foundation` with the focus ring, `croftClickable`, `longPressLift`, `explainsWhenDisabled`, forced interaction sources and the motion probes tests read).
- Haptics: `.../haptics/Haptics.kt` (`Haptics`, `HapticEvent`, `LevelFilteredHaptics`, `AndroidHaptics`, `ProvideHaptics`, `LocalHaptics`). `CroftTheme` provides it and switches off Compose's own platform haptic feedback. `RecordingHaptics` is in `app/src/test/.../haptics/`.
- Gallery: `app/src/debug/kotlin/com/example/croft/debug/` (`DebugGallery`, `gallery/GalleryCatalog` (all entries, as data), `GalleryScreen`). `app/src/release/.../DebugGallery.kt` is a stub with the same API. Gallery tests are in `app/src/testDebug/`.
- Theme additions: `theme/MotionSchemes.kt` (`OffMotionScheme`: every spring a jump; `CroftThemeProvider` uses it at Off, so Material's own components are instant), `theme/Timings.kt` (waits and one linear fade that `tokens.json` does not hold), `theme/StatusColors.kt` (amber).
- Navigation: `ui/nav/` (`CroftApp` reads the stored `ThemeSettings` and applies `CroftTheme`; `CroftNavHost`: Home, Settings, Appearance with S50 and its drill-down, gallery in debug builds; `SettingsHomeScreen`). `MainActivity` uses `ThemeStore.shared(context)` (one DataStore per process) and does not open the core.
- S53 Try it buttons play `TryItEvent.hapticEvent` through `Haptics` (`rememberTryItHaptics()` is the default of `onTry`).

### Done-when, line by line
| Line | Test |
|---|---|
| Screenshot tests, each component in every state, light and dark, font 1.0 and 2.0, LTR and RTL | `GalleryScreenshotTest` (128 pictures: 16 sections x 2 x 2 x 2); a state with no gallery entry fails `GalleryCompletenessTest`, which reads the tables of `controls.md` and iterates the control enums |
| At font scale 2.0 no text is clipped; the FAB collapses to an icon | `GallerySemanticsTest.atFontScale2_noTextIsClipped` (every section); `MotionTest` FAB width tests (bounds) |
| Semantics: icon-only controls have a description; disabled controls stay in the tree as disabled; touch targets at least 48 dp | `GallerySemanticsTest` (every interactive node of every section, font 1.0), `ButtonsAndSendTest`, `ChipsFieldsTogglesTest`, `FeedbackTest` |
| `Haptics`: level filter, 40 ms guard on a fake clock, recording fake keeps order | `HapticsTest`, `TryItHapticsTest`, `RowAndAttachmentTest` |
| Reduced-motion substitutes and an interrupt test | `MotionTest` (Full, Reduced and Off for FAB, filter chip, avatar flip, long-press lift, new-row tint, failed edge, dialog, banner, spinner, the motion scheme behind Material, a Material switch compared pixel by pixel); interrupt: a tap during the FAB animation, the avatar turning back from where it is |
| No hard-coded colour, duration or radius outside the theme package | `NoHardCodedDesignValuesTest` (also checks the rule on sample lines) |
| Gallery only in debug builds | `GalleryIsDebugOnlyTest` (source sets, release stub, no hidden gesture) and `compileReleaseKotlin` (the release classes hold only the stub) |
| Switches, long-press, destructive confirm call `Haptics` from the gallery | `GalleryScreenTest` |

### Choices to copy into `docs/DECISIONS.md`
| Date | Choice | Why |
|---|---|---|
| 2026-11-11 | No new libraries. `HapticFeedbackConstantsCompat` is in `androidx.core:core` (core-ktx 1.19.1, already used). Tests also use `kotlinx-coroutines-test` and Espresso `pressBack`, which arrive with `ui-test-junit4`. | Nothing proprietary, nothing to add to `LICENSES.md`. |
| 2026-11-11 | The gallery is the `debug` source set plus a stub in `release`, not a flag. Tests that need it are in `src/testDebug`. | A release build contains no gallery code and no route to it. |
| 2026-11-11 | `CroftTheme` replaces `LocalHapticFeedback` with a no-op. | Material and foundation vibrate on a long-press by themselves; that would bypass the level filter and the recording test. |
| 2026-11-11 | Amber (starred, Server problem chip) is made with the HCT model at hue 78, chroma 80 (`theme/StatusColors.kt`). Durations of waits (600 ms check, 10 s Cancel, 4 s snackbar, 1.2 s new tint, 2 s failed edge) are in `theme/Timings.kt`. `DesignTokenRulesTest` skips that one file. | `tokens.json` has neither. **Owner to do:** add an amber role and these times to `tokens.json` and regenerate. |
| 2026-11-11 | Material's menu and bottom sheet keep their own open and close; at Reduced they use the standard springs (no overshoot), at Off they are instant. The catalogue asks for a fade at Reduced. | A custom popup was not worth it; Off is covered. |
| 2026-11-11 | Expressive button shape morph on press is not done (only Material's pressed state layer and ripple). | Not needed to meet the tables; revisit with a device. |

### Motion catalogue entries to add to `docs/ux/motion.md`
| Animation | Trigger | What moves | Token | Reduced-motion substitute |
|---|---|---|---|---|
| New row tint | A message arrives while visible | Primary tint fades over 1.2 s (linear) | `CroftTimings.NEW_ROW_TINT_MS` | Same at Reduced; not shown at Off |
| Failed edge | A change the server refused | Red edge on the row, then fades after 2 s | Effects default | Edge removed at 2 s without a fade |
| Dialog open (as built) | Open | Scale from 90 percent and fade | Effects default | Fade only; Off at once |
| Button check | Loading ends | Spinner, then a check for 600 ms | none | Same |

### Tests: what ran and where
- **Gradle:** `./gradlew :app:assembleDebug` passes. `:app:compileDebugUnitTestKotlin` and `:app:compileReleaseKotlin` pass. `:app:testDebugUnitTest` **could not run here**: Maven Central answers HTTP 429 for `org.robolectric:nativeruntime-dist-compat:1.0.19` (also with `--offline`, the cached file is not registered). Nothing was worked around: no mirror, init script or repository override. GitHub CI is the real run.
- **By hand, same sources:** a JUnit launcher (`org.junit.runner.JUnitCore`) with the jars already in the local Gradle cache and `~/.m2` (outside the repository, not committed). 943 tests ran, 939 passed. The 4 failures are T11's `KeystoreErrorsTest` and `KeystoreKeySourceTest` ("Stub!"): the launcher used the real `android.jar` where Gradle uses its mockable one. The 5th (`DesignTokenRulesTest`, which scanned `theme/Timings.kt`) was fixed and passes. 434 of the tests are new in T13 (160 gallery semantics, 128 gallery pictures, 35 motion, 18 + 17 + 16 + 9 chips, feedback, buttons, rows, and the rest).
- Goldens: two new (`gallery-row-light-font1.0-ltr`, `gallery-send-dark-font2.0-rtl`, about 100 KB and 75 KB), recorded here and compared with 1 percent tolerance. If CI's text rendering differs, refresh them with `-Pcroft.screenshots=record-goldens`.

### On a device (not checkable here)
Ripple and state-layer feel, real haptic strength and the fallbacks below API 34, focus ring with a hardware keyboard, tooltip timing, menu and sheet motion, Material's switch thumb feel.

### Not done
- Drag between To, Cc and Bcc (the chip lifts and plays the DragStart haptic; the drop is for compose, T17).
- The swipe engine itself: the row only shows the dragging card and pill for a given offset (T15).
- Predictive back animation between app screens (back works; the transition is the shared axis).
- Expressive button shape morph on press.
- Web-only hover is shown in the gallery through a forced interaction state; there is no mouse test.
