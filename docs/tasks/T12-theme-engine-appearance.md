# T12: Theme engine and Appearance screens

Crate / folder: `android/` (app and a shared UI module if needed)

Depends on: T08 (generated tokens), T07 (the app skeleton). Independent of the core, so it can run in parallel with T09 to T11. Blocks: T13 onwards (every screen uses the theme and the screenshot setup chosen here).

## Read first
`docs/ux/build-order.md` (step 2 and "Checks that apply to every step"), `docs/ux/theme.md` (all of it), `docs/ux/customisation.md` (Look and Feel rows), `docs/ux/motion.md`, `docs/ux/accessibility.md` (contrast), `design/CLAUDE.md`, `design/tokens.json`, `design/generated/kotlin/CroftTokens.kt`, `design/wireframes/` for S08, S50 to S53, `android/README.md`.

## Goal
Theme engine and the screens S50 (Appearance), S51 (Custom colour), S52 (Layout and density), S53 (Motion and haptics, settings only; the `Haptics` interface comes in T13) and S08 (Pick your look), all on fake data.

## Spec
- A `ThemeSettings` data class equal to the stored shape in `docs/ux/theme.md`, persisted as JSON (DataStore), unreadable or newer-version files fall back to Moss, Standard, System. Settings reach every composable through one `CompositionLocal`.
- Presets, modes (System, Light, Dark, Pure black) and contrast levels come from the generated tokens. For a custom seed, style or contrast the app computes the scheme at runtime with Material Color Utilities (the Kotlin port `com.google.android.material:material-color-utilities`, same version family as the TypeScript one T08 pinned). Wallpaper colours use `dynamicColorScheme` on Android 12 and later and the option is hidden below that.
- Cross-check: the runtime result for each preset's seed must equal the generated table (T08 golden values). Any difference is a bug in this task, not in the tokens.
- Live preview on S50, S08 and S51 is built from real components (search pill, two rows, chip, FAB, snackbar) at preview size, non-interactive.
- Change of theme cross-fades with the Effects slow spring and does not move layout. Reset appearance asks first; applying a palette shows an Undo snackbar for 5 s.
- Export theme writes `croft-theme.json`; Import reads it and rejects a bad file with a plain message. Sharing as `croft://theme?...` is optional in this task; leave the link format and QR to a later task unless it is trivial.
- S52 controls: density, roundness (scales radii by 0.6, 1, 1.4, capped at half the height), list style, avatars, unread emphasis, text size 85 to 150 percent, fonts (Croft Sans must be an open-licence font added to the licence list; Atkinson Hyperlegible likewise).
- Screenshot tooling: choose a JVM-run library (Roborazzi on Robolectric native graphics, or Paparazzi), check it works with the AGP and Compose versions in `docs/DECISIONS.md`, and record the choice and golden-image location in `docs/DECISIONS.md`. T13 to T21 reuse it. If Maven Central still rate-limits the container (`docs/ENVIRONMENT.md`), say which checks ran only on CI.

## Done when
- Unit test: for every preset (8) x mode (light, dark, pure black) x contrast (standard, medium, high), every text/icon role against its container is at least 4.5 to 1 (3 to 1 for large text and graphics; 7 to 1 at High). Test names list the failing pair.
- Unit test: runtime-computed colours equal the generated golden values for all presets and modes.
- Export then import returns an identical file; a corrupt, empty and too-new file each fall back safely (tests).
- Screenshot tests: preview and S10-style sample for every preset in light and dark; S50, S51, S52, S53, S08 in compact, medium and expanded widths, light and dark, font scale 1.0 and 2.0, LTR and RTL.
- A Compose test proves a theme change does not change the layout bounds of the preview.
- Reduced-motion test: with Animation level Reduced or Off the theme change is a plain cross-fade or instant.
- On a device (not checkable here): that the cross-fade looks smooth, wallpaper colours on a real Android 12+ phone, and the text-size slider feel.
