## Theme picker

The theme picker (S08 in onboarding, S50 in settings) lets the person change the whole look of Croft in a few taps and see the result before leaving the screen. It has four layers, each independent: **mode**, **colour**, **shape and density**, **type**. Motion and haptics sit beside it on S53.

### What the screen contains

1. **Live preview** pinned at the top: a real, non-interactive inbox built from the actual components (search pill, two rows, chip, FAB, snackbar), not a picture. Every change below updates it within a frame.
2. **Mode**: System, Light, Dark, and a Pure black switch for OLED screens (shown only in dark).
3. **Colour source**: a row of choices, each one a small swatch tile.
4. **Palettes**: the Croft presets, shown as tiles with the primary, secondary and surface colours.
5. **Style**: how colourful the scheme is (see variants below).
6. **Contrast**: Standard, Medium, High.
7. Links to **Layout and density** (S52), **Custom colour** (S51) and **Motion and haptics** (S53).

### Colour sources

| Source | How it works | Availability |
| --- | --- | --- |
| Croft palette | One of the presets below | Everywhere |
| Wallpaper | Colours drawn from the phone's wallpaper through Android's dynamic colour | Android 12 and later |
| Custom colour | A seed chosen by wheel, hex value or a photo (S51) | Everywhere |
| Per-account colour | Not a theme; each account has a colour used for its avatar stripe, status chip and the From chip | Everywhere |

The default is the Croft palette Moss, not the wallpaper, so the app looks the same on every phone and in screenshots. Wallpaper colours are one tap away, and the onboarding picker offers them first on Android 12 and later.

### Croft palettes

Each preset is a seed colour plus a style. Names are plain words, not product-y ones. Seeds are starting values, to be checked on screen.

| Preset | Seed | Feel |
| --- | --- | --- |
| Moss (default) | `#4F6F52` | Calm green, paper-white surfaces in light |
| Wax | `#B4472E` | Warm red, the accent of the brand |
| Slate | `#4A6275` | Cool blue-grey, quiet and neutral |
| Heather | `#77607F` | Muted purple, no gradients |
| Ochre | `#A9741B` | Warm amber, high contrast |
| Peat | `#6B5B4B` | Brown-grey, very low saturation |
| Tide | `#2F6F73` | Teal, slightly cooler than Moss |
| Rowan | `#9A3F52` | Deep berry |

### Scheme generation

- Schemes come from the Material Color Utilities (the same HCT colour model Android's dynamic colour uses): a seed produces tonal palettes and then every colour role for light and dark.
- **Styles**: Tonal spot (default, calm), Neutral (nearly grey), Vibrant, Expressive, Fidelity (stays close to the seed), Monochrome.
- **Contrast levels** use the library's contrast setting: Standard, Medium and High. High contrast is also chosen automatically when the system's high-contrast text setting is on, unless the person has picked a level.
- **Pure black** replaces the darkest surface with `#000000` and keeps the raised surface containers a few tone steps lighter so cards and the list container remain visible.
- The same library exists for Kotlin, TypeScript and Swift. A golden test feeds each preset's seed to all three and requires identical colour roles, so the platforms cannot drift.
- A custom seed with almost no colour (very low chroma) is allowed; the library handles it. The picker shows a note when the chosen style makes the scheme look grey.

### Shape and density (S52)

| Setting | Options | Default | Effect |
| --- | --- | --- | --- |
| Density | Comfortable, Default, Compact | Default | Comfortable: about 88 dp rows, avatar 44, two preview lines. Default: about 72 dp, avatar 40, one preview line. Compact: about 56 dp, avatar 32, no preview |
| Roundness | Tight, Standard, Round | Standard | Scales the corner radii of containers, buttons and chips by 0.6, 1 and 1.4, limited to half the component height. The swipe card's corner is 17 dp at Standard, so 10 at Tight and 24 at Round |
| List style | Container, Flat | Container | Container: the list sits in one rounded surface on a tinted background, as in current Gmail. Flat: rows run edge to edge on the plain background |
| Avatars | Letters, Photos, Hidden | Letters | Photos come from contacts and BIMI logos if the person allows it |
| Unread emphasis | Bold and dot, Bold, Dot | Bold and dot | Never colour alone |
| Show previews | Off, 1 line, 2 lines | By density |  |

Row heights are design starting points to adjust on a phone.

### Type

- **Text size**: a slider from 85 to 150 percent with ticks, multiplied with the system font size; layouts must hold at the combined 200 percent without clipping.
- **Interface font**: System (default), Croft Sans (a bundled open-licence sans) or Atkinson Hyperlegible.
- **Reading font** for message bodies: System, Serif, or Atkinson Hyperlegible. Does not change the interface.
- Message bodies from HTML mail are shown as sent with the reading font size applied. In dark mode only simple light-background mails are inverted; complex HTML mail shows on a light card instead of being recoloured badly.

### Behaviour

- **Preview first**: tapping a palette changes the preview and the app together with an effects slow cross-fade. A snackbar offers Undo for 5 s, so a bad choice is one tap from recovery.
- **Revert**: Reset appearance is in the overflow menu and asks first.
- **Safe fallback**: if the stored theme cannot be read, Croft starts with Moss, Standard, System mode.
- **Share and back up**: Export theme saves `croft-theme.json`; Import reads one; Share offers a `croft://theme?...` link and a QR code. With a Companion connected, appearance syncs between the person's devices; without one it stays on the device.
- **Web**: the same tokens become CSS custom properties; `color-scheme` and the browser's theme colour follow the mode; the picker is the same layout in a settings pane (W04).

### Stored shape

```json
{
  "version": 1,
  "mode": "system",
  "pureBlack": false,
  "source": "preset",
  "preset": "moss",
  "seed": "#4F6F52",
  "style": "tonalSpot",
  "contrast": "standard",
  "density": "default",
  "roundness": "standard",
  "listStyle": "container",
  "avatars": "letters",
  "unreadEmphasis": "boldDot",
  "textScale": 1.0,
  "uiFont": "system",
  "readingFont": "system",
  "animation": "system",
  "haptics": "subtle",
  "accountColors": { "acct-1": "#4F6F52" }
}
```

### Acceptance checks

- For every preset, in light, dark and pure black, at all three contrast levels, text and icon colours against their containers meet 4.5 to 1 (3 to 1 for large text and graphics), and 7 to 1 at High contrast. This is an automated test on the generated roles.
- Screenshot tests of the preview and of S10 for every preset in light and dark.
- Export then import returns an identical file.
