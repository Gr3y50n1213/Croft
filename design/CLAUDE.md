# Design: rules for every session that touches a screen

`tokens.json` is the single source of colours, motion, shapes, spacing and swipe geometry. A generator
(`design/generator/`) turns it into the files every platform uses. Read `docs/ux/theme.md`, `docs/ux/motion.md`
and `docs/ux/haptics.md` before changing anything here.

## Rules
- **Use the tokens.** Never hard-code a colour, a duration, a spring or a radius in Kotlin, Swift, CSS or TypeScript.
  Take colours from the theme roles, motion from the motion scheme or spring tokens, corners from the shape tokens.
- **Every new control gets an entry in the component gallery** (build-order step 3), in every state.
- **Every new animation gets a reduced-motion variant** and an interrupt test, and is added to the catalogue in
  `docs/ux/motion.md`.
- **Every haptic goes through the `Haptics` interface**, never a direct platform call, so the recording test sees it.
- A new value belongs in `tokens.json` first. Do not edit anything in `generated/` by hand.

## Layout
- `tokens.json`: the source. Keys starting with `_` are notes and are not emitted.
- `generator/`: Node and TypeScript. Colour schemes come from `@material/material-color-utilities` (HCT); no colour
  maths is written here.
- `generated/`: committed output. `kotlin/CroftTokens.kt`, `css/croft-tokens.css`, `swift/CroftTokens.swift`,
  `ts/croft-tokens.ts`. Generation is deterministic, so unchanged tokens give byte-identical files.
- `wireframes/`, `screens.csv`: structure and the screen inventory.

## Regenerate and test
```
cd design/generator && npm ci && npm run generate && npm test
git status --porcelain design/generated   # commit whatever changed
```
`npm test` fails if `generated/` does not match `tokens.json`, if the four outputs disagree on any colour role,
or if a spring differs from the table in `docs/ux/motion.md`.

## What the outputs contain
- Colour roles for every preset (Moss, Wax, Slate, Heather, Ochre, Peat, Tide, Rowan) in light, dark and pure black,
  at Standard, Medium and High contrast. Presets use the Tonal spot style unless `palettes.styles` in `tokens.json`
  names another. A custom seed from the picker is not precomputed: the apps run the Material Color Utilities
  library at runtime for it.
- Pure black is the dark scheme with `background`, `surface`, `surfaceDim` and `surfaceContainerLowest` at `#000000`.
- Kotlin is written for Compose Material 3 1.5.x (its `MotionScheme` is public from 1.5). Package
  `com.example.croft.design` is a placeholder until the real application id is chosen.
- CSS: `[data-croft-theme="moss"][data-croft-mode="dark"]` (modes `light`, `dark`, `pure-black`), plus
  `[data-croft-contrast="medium|high"]`. Lengths are px (one dp is one px).
