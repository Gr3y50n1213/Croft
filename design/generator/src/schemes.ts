// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Colour roles from Material Color Utilities (HCT). Nothing here does colour maths: the library does.

import {
  Hct,
  TonalPalette,
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

// ---- Status colours (amber): a fixed HCT hue and chroma read at tones set in tokens.json ----

export const STATUS_MODES = ['light', 'dark', 'pureBlack'] as const;

export interface StatusRoles {
  color: string;
  container: string;
  onContainer: string;
}

export interface StatusEntry {
  name: string;
  mode: Mode;
  contrast: ContrastId;
  colors: StatusRoles;
}

type ToneSet = { color: number; container: number; onContainer: number };

function toneSet(raw: unknown, where: string): ToneSet {
  const t = raw as Record<string, unknown> | undefined;
  const out = {} as ToneSet;
  for (const k of ['color', 'container', 'onContainer'] as const) {
    const v = t?.[k];
    if (typeof v !== 'number' || v < 0 || v > 100) throw new Error(`status ${where}.${k}: needs a tone from 0 to 100`);
    out[k] = v;
  }
  return out;
}

/** One entry per status colour, mode and contrast. Pure black uses the dark tones. Not preset-dependent. */
export function buildStatusColors(tokens: Tokens): StatusEntry[] {
  const entries: StatusEntry[] = [];
  for (const [name, raw] of Object.entries(tokens.status ?? {})) {
    if (name.startsWith('_')) continue;
    const def = raw as { hue?: unknown; chroma?: unknown; tones?: Record<string, Record<string, unknown>> };
    if (typeof def.hue !== 'number' || typeof def.chroma !== 'number') throw new Error(`status ${name}: needs hue and chroma`);
    const palette = TonalPalette.fromHueAndChroma(def.hue, def.chroma);
    for (const mode of STATUS_MODES) {
      const toneMode = mode === 'light' ? 'light' : 'dark';
      for (const contrast of CONTRASTS) {
        const tones = toneSet(def.tones?.[toneMode]?.[contrast.id], `${name}.tones.${toneMode}.${contrast.id}`);
        const hex = (tone: number): string => hexFromArgb(palette.tone(tone)).toUpperCase();
        entries.push({
          name, mode, contrast: contrast.id,
          colors: { color: hex(tones.color), container: hex(tones.container), onContainer: hex(tones.onContainer) },
        });
      }
    }
  }
  return entries;
}

export function statusNames(entries: StatusEntry[]): string[] {
  return [...new Set(entries.map((e) => e.name))];
}
