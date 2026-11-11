# T08: Design token generator

Crate / folder: `design + android`

## Goal
Step 1 of the UX build order (docs/ux/build-order.md).

## Spec
- Treat `design/tokens.json` as the source. Generate a Kotlin file (Compose theme, `MotionScheme` wrappers), CSS variables and a Swift file.
- Colour schemes come from Material Color Utilities (HCT) given a seed, style and contrast level. Port or call the reference implementation; do not hand-roll HCT.
- Golden tests: the same seed produces identical colours in Kotlin, TypeScript and Swift output.

## Done when
- Golden test passes for all eight presets in light, dark and pure black.
- Changing one value in `tokens.json` changes all three outputs.
