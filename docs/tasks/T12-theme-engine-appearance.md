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

## Result (session notes, T12)

### Where things are
- Theme engine: `android/app/src/main/kotlin/com/example/croft/theme/` (`ThemeSettings`, `ThemeJson`, `ThemeStore`, `ThemeEngine`, `McuSchemes`, `CroftThemeProvider`, `ScaledShapes`, `Contrast`). `CroftTheme` in `com/example/croft/CroftTheme.kt` now takes `ThemeSettings` and delegates to it.
- Screens: `.../ui/appearance/` (`AppearanceScreen` S50, `CustomColourScreen` S51, `LayoutDensityScreen` S52, `MotionHapticsScreen` S53, `PickYourLookScreen` S08, `AppearanceFlow` for the drill-down, `AppearancePreview` and `InboxSample`). They are not linked from the app yet: T13 adds the navigation and the settings entry; `MainActivity` is unchanged. `ThemeStore.create(context)` is the DataStore to collect there.
- Strings: `res/values/strings_appearance.xml`. Fonts: `res/font/` (Croft Sans = Inter Regular, Medium, Bold; Atkinson Hyperlegible Regular, Bold), licence texts in `assets/licenses/` (both SIL OFL 1.1, no reserved font names).

### Choices to copy into `docs/DECISIONS.md`
| Date | Choice | Why |
|---|---|---|
| 2026-11-11 | Runtime colour: `com.google.android.material:material:1.14.0` (Apache-2.0), only its `com.google.android.material.color.utilities` classes, wrapped in `theme/McuSchemes.kt`. The artifact `com.google.android.material:material-color-utilities` named in this task does not exist. | Google's own Java port of the library (same HCT model as Android dynamic colour). Reproduces all 72 generated preset tables exactly (test). The artifact pulls the AndroidX Views libraries transitively (its resources need them to link); R8 removes what is unused. The third-party Kotlin port `com.materialkolor:material-color-utilities` was not used (MIT wrapper, not Google's code). |
| 2026-11-11 | Screenshot tests: Roborazzi 1.78.0 (`roborazzi`, `roborazzi-compose`, Apache-2.0) on Robolectric 4.17 native graphics, library only (no Gradle plugin). Paparazzi only has `2.0.0-alpha05`. Pictures go to `android/app/build/outputs/screenshots/` (not committed); 8 golden pictures are committed in `android/app/src/test/screenshots/` and compared with a 1 percent tolerance; refresh with `./gradlew :app:testDebugUnitTest -Pcroft.screenshots=record-goldens`. Helper: `android/app/src/test/kotlin/com/example/croft/ui/ScreenshotKit.kt`. | Roborazzi runs on the Robolectric already used by the project, needs no emulator, and works without its Gradle plugin, so AGP 9.5 compatibility of the plugin does not matter. |
| 2026-11-11 | Settings file: `croft-theme.json` is the stored shape of `docs/ux/theme.md` plus a leading `"version": 1`. A file without `version` counts as 1; a higher version is refused. | Needed to tell a newer file from a corrupt one. `docs/ux/theme.md` should show the field. |
| 2026-11-11 | Other new libraries: `androidx.datastore:datastore:1.3.0`, `org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0` (JSON values only, no compiler plugin), `androidx.compose.material:material-icons-core` (BOM version). All Apache-2.0. | Spec asks for DataStore; JSON is needed for export and import; the icon core set has Back, Check, More. |

### Licence list (add to `LICENSES.md`)
Inter 3.x (Croft Sans), Copyright 2020 The Inter Project Authors, SIL OFL 1.1; Atkinson Hyperlegible, Copyright 2020 Braille Institute of America, Inc., SIL OFL 1.1. Both texts are in `android/app/src/main/assets/licenses/`. The font files came from the `@expo-google-fonts/inter` 0.4.2 and `@expo-google-fonts/atkinson-hyperlegible` 0.4.1 npm packages (the TTFs of the Google Fonts release).

### Motion catalogue entries to add to `docs/ux/motion.md`
| Animation | Trigger | What moves | Token | Reduced-motion substitute |
|---|---|---|---|---|
| Try it stage (S53) | Press a Try it button | Square pops from 70 percent to 100 percent | Spatial default | Short fade (Effects default); Off: nothing |
| Appearance drill-down (S50 to S51, S52, S53) | Open or close | Shared axis X | Spatial default (move), Effects default (fade) | Cross-fade; Off: instant |
Theme change itself is already in the catalogue (Effects slow; Reduced and Off swap at once).

### Not done in this task
- High contrast chosen automatically when the system's high-contrast text is on (needs a hidden platform setting; the contrast level is only what the person picks).
- `croft://theme?...` link and QR (optional).
- Wiring `ThemeStore` into `MainActivity` and a settings entry for S50 (T13).
- The `Haptics` interface: S53 Try it buttons call `onTry(TryItEvent)`, which T13 connects.
- "Off" makes the theme change and the drill-down instant; the Material components' own springs still run at Off until every component reads `LocalMotionLevel` (T13 gallery).
