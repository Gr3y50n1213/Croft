// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Colour roles from Material Color Utilities (HCT). Nothing here does colour maths: the library does.

import {
  Hct,
  SchemeExpressive,
  SchemeFidelity,
  SchemeMonochrome,
  SchemeNeutral,
  SchemeTonalSpot,
  SchemeVibrant,
  argbFromHex,
  hexFromArgb,
} from '@material/material-color-utilities';
import type { DynamicScheme } from '@material/material-color-utilities';
import { isHex } from './tokens.ts';
import type { Tokens } from './tokens.ts';

/** Roles in the order of the Compose Material 3 `lightColorScheme` parameters. */
export const ROLES = [
  'primary', 'onPrimary', 'primaryContainer', 'onPrimaryContainer', 'inversePrimary',
  'secondary', 'onSecondary', 'secondaryContainer', 'onSecondaryContainer',
  'tertiary', 'onTertiary', 'tertiaryContainer', 'onTertiaryContainer',
  'background', 'onBackground', 'surface', 'onSurface', 'surfaceVariant', 'onSurfaceVariant',
  'surfaceTint', 'inverseSurface', 'inverseOnSurface',
  'error', 'onError', 'errorContainer', 'onErrorContainer',
  'outline', 'outlineVariant', 'scrim',
  'surfaceBright', 'surfaceContainer', 'surfaceContainerHigh', 'surfaceContainerHighest',
  'surfaceContainerLow', 'surfaceContainerLowest', 'surfaceDim',
  'primaryFixed', 'primaryFixedDim', 'onPrimaryFixed', 'onPrimaryFixedVariant',
  'secondaryFixed', 'secondaryFixedDim', 'onSecondaryFixed', 'onSecondaryFixedVariant',
  'tertiaryFixed', 'tertiaryFixedDim', 'onTertiaryFixed', 'onTertiaryFixedVariant',
] as const;
export type Role = (typeof ROLES)[number];
export type RoleColors = Record<Role, string>;

export const MODES = ['light', 'dark', 'pureBlack'] as const;
export type Mode = (typeof MODES)[number];

/** The library's contrast setting: Standard 0, Medium 0.5, High 1 (docs/ux/theme.md). */
export const CONTRASTS = [
  { id: 'standard', level: 0 },
  { id: 'medium', level: 0.5 },
  { id: 'high', level: 1 },
] as const;
export type ContrastId = (typeof CONTRASTS)[number]['id'];

/** The six styles named in docs/ux/theme.md (Fidelity is the library's SchemeFidelity). */
const STYLES = {
  tonalSpot: SchemeTonalSpot,
  neutral: SchemeNeutral,
  vibrant: SchemeVibrant,
  expressive: SchemeExpressive,
  fidelity: SchemeFidelity,
  monochrome: SchemeMonochrome,
} as const;
export type StyleId = keyof typeof STYLES;
export const DEFAULT_STYLE: StyleId = 'tonalSpot';

/** Pure black: roles at or below the base surface tone become #000000. Containers keep their tones. */
export const PURE_BLACK_ROLES: readonly Role[] = ['background', 'surface', 'surfaceDim', 'surfaceContainerLowest'];

export interface Preset {
  id: string;
  seed: string;
  style: StyleId;
}

export interface SchemeEntry {
  preset: string;
  mode: Mode;
  contrast: ContrastId;
  colors: RoleColors;
}

export function presetsOf(tokens: Tokens): Preset[] {
  const defaultStyle = tokens.palettes.defaultStyle ?? DEFAULT_STYLE;
  return Object.entries(tokens.palettes.presets).map(([id, seed]) => {
    if (!isHex(seed)) throw new Error(`preset ${id}: seed "${seed}" is not #RRGGBB`);
    const style = tokens.palettes.styles?.[id] ?? defaultStyle;
    if (!(style in STYLES)) throw new Error(`preset ${id}: unknown style "${style}"`);
    return { id, seed: seed.toUpperCase(), style: style as StyleId };
  });
}

function rolesOf(scheme: DynamicScheme): RoleColors {
  const out = {} as RoleColors;
  for (const role of ROLES) out[role] = hexFromArgb(scheme[role]).toUpperCase();
  return out;
}

export function buildScheme(preset: Preset, mode: Mode, contrastLevel: number): RoleColors {
  const SchemeClass = STYLES[preset.style];
  const scheme = new SchemeClass(Hct.fromInt(argbFromHex(preset.seed)), mode !== 'light', contrastLevel);
  const colors = rolesOf(scheme);
  if (mode === 'pureBlack') {
    for (const role of PURE_BLACK_ROLES) colors[role] = '#000000';
  }
  return colors;
}

export function buildAllSchemes(tokens: Tokens): SchemeEntry[] {
  const entries: SchemeEntry[] = [];
  for (const preset of presetsOf(tokens)) {
    for (const mode of MODES) {
      for (const contrast of CONTRASTS) {
        entries.push({ preset: preset.id, mode, contrast: contrast.id, colors: buildScheme(preset, mode, contrast.level) });
      }
    }
  }
  return entries;
}
