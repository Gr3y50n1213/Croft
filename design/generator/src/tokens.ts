// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Reading design/tokens.json and turning it into a flat list of leaves.
// Keys that start with an underscore are notes, not tokens, and never reach an output.

import { readFileSync } from 'node:fs';

export type Scalar = number | string | boolean;
export type LeafValue = Scalar | Scalar[];

export interface Leaf {
  path: string[];
  value: LeafValue;
}

export interface Tokens {
  brand: Record<string, string>;
  palettes: {
    default: string;
    presets: Record<string, string>;
    /** Optional per-preset style. theme.md names no style per preset, so it is normally absent. */
    styles?: Record<string, string>;
    /** Optional style for presets without an entry in `styles`. Defaults to tonalSpot. */
    defaultStyle?: string;
  };
  /** Status colours (amber, ...): HCT hue and chroma plus tones per mode and contrast. */
  status?: Record<string, unknown>;
  [section: string]: unknown;
}

/** Sections handled by dedicated code rather than the generic leaf emitters. */
const SPECIAL_SECTIONS = new Set(['brand', 'palettes', 'status']);

export function loadTokens(path: string): Tokens {
  const parsed: unknown = JSON.parse(readFileSync(path, 'utf8'));
  if (typeof parsed !== 'object' || parsed === null) throw new Error(`${path}: not a JSON object`);
  const tokens = parsed as Tokens;
  if (!tokens.brand || !tokens.palettes?.presets) throw new Error(`${path}: needs "brand" and "palettes.presets"`);
  if (!tokens.palettes.presets[tokens.palettes.default]) {
    throw new Error(`${path}: palettes.default "${tokens.palettes.default}" is not one of palettes.presets`);
  }
  return tokens;
}

/** The nested object of generic (non-colour-scheme) tokens, with note keys removed. */
export function genericTree(tokens: Tokens): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(tokens)) {
    if (key.startsWith('_') || SPECIAL_SECTIONS.has(key)) continue;
    out[key] = stripNotes(value);
  }
  return out;
}

function stripNotes(value: unknown): unknown {
  if (Array.isArray(value)) return value;
  if (typeof value === 'object' && value !== null) {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) {
      if (!k.startsWith('_')) out[k] = stripNotes(v);
    }
    return out;
  }
  return value;
}

export function flatten(tree: Record<string, unknown>, prefix: string[] = []): Leaf[] {
  const leaves: Leaf[] = [];
  for (const [key, value] of Object.entries(tree)) {
    const path = [...prefix, key];
    if (Array.isArray(value)) {
      leaves.push({ path, value: value as Scalar[] });
    } else if (typeof value === 'object' && value !== null) {
      leaves.push(...flatten(value as Record<string, unknown>, path));
    } else {
      leaves.push({ path, value: value as Scalar });
    }
  }
  return leaves;
}

export function pascal(word: string): string {
  return word.charAt(0).toUpperCase() + word.slice(1);
}

export function kebab(word: string): string {
  return word
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1-$2')
    .toLowerCase();
}

export function upperSnake(word: string): string {
  return kebab(word).replace(/-/g, '_').toUpperCase();
}

export function isHex(value: string): boolean {
  return /^#[0-9A-Fa-f]{6}$/.test(value);
}
