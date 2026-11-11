// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Parsers that read the generated Kotlin, Swift, CSS and TypeScript back into plain data,
// so tests compare what a platform would actually see.

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { OUTPUT_FILES } from '../src/generate.ts';
import type { Leaf, LeafValue } from '../src/tokens.ts';

export const here = dirname(fileURLToPath(import.meta.url));
export const designDir = resolve(here, '../..');
export const repoRoot = resolve(designDir, '..');
export const tokensPath = join(designDir, 'tokens.json');
export const committedDir = join(designDir, 'generated');

/** key "preset/mode/contrast" (ids as in tokens: moss/pureBlack/standard) -> role -> "#RRGGBB" */
export type ParsedSchemes = Map<string, Record<string, string>>;

const norm = (s: string): string => s.toLowerCase().replace(/[_-]/g, '');
const lowerFirst = (s: string): string => s.charAt(0).toLowerCase() + s.slice(1);

const MODE_IDS: Record<string, string> = { light: 'light', dark: 'dark', pureblack: 'pureBlack' };
function modeId(raw: string): string {
  const id = MODE_IDS[norm(raw)];
  if (!id) throw new Error(`unknown mode ${raw}`);
  return id;
}

function parseLiteral(raw: string): LeafValue {
  const text = raw.trim();
  const list = /^(?:listOf\(|\[)(.*?)\)?\]?$/.exec(text);
  if (/^(listOf\(.*\)|\[.*\])$/.test(text) && list) {
    const inner = (list[1] ?? '').trim();
    return inner === '' ? [] : inner.split(',').map((x) => parseLiteral(x) as number | string | boolean);
  }
  if (text === 'true') return true;
  if (text === 'false') return false;
  if (text.startsWith('"')) return JSON.parse(text) as string;
  const n = Number(text);
  if (Number.isNaN(n)) throw new Error(`cannot parse literal ${raw}`);
  return n;
}

/** Walks the `object|enum CroftTokens { ... }` region and returns its leaves. */
function parseNestedTokens(text: string, keyword: 'object' | 'enum', leafPattern: RegExp): Leaf[] {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => l === `${keyword} CroftTokens {`);
  if (start < 0) throw new Error('CroftTokens region not found');
  const stack: string[] = [];
  const leaves: Leaf[] = [];
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i] as string;
    const open = new RegExp(`^\\s*${keyword} (\\w+) \\{$`).exec(line);
    if (open) {
      stack.push(lowerFirst(open[1] as string));
    } else if (/^\s*\}$/.test(line)) {
      if (stack.length === 0) return leaves;
      stack.pop();
    } else {
      const m = leafPattern.exec(line);
      if (m) leaves.push({ path: [...stack, (m[1] as string).replace(/`/g, '')], value: parseLiteral(m[2] as string) });
    }
  }
  throw new Error('CroftTokens region not closed');
}

export interface Parsed {
  schemes: ParsedSchemes;
  leaves: Leaf[];
  brand: Record<string, string>;
}

export function parseKotlin(text: string): Parsed {
  const schemes: ParsedSchemes = new Map();
  const re =
    /CroftSchemeKey\(CroftPreset\.(\w+), CroftMode\.(\w+), CroftContrast\.(\w+)\) to CroftColorRoles\(([^)]*)\)/g;
  for (const m of text.matchAll(re)) {
    const roles: Record<string, string> = {};
    for (const r of (m[4] as string).matchAll(/(\w+) = 0xFF([0-9A-F]{6}),/g)) roles[r[1] as string] = `#${r[2]}`;
    schemes.set(`${norm(m[1] as string)}/${modeId(m[2] as string)}/${norm(m[3] as string)}`, roles);
  }
  const leaves = parseNestedTokens(text, 'object', /^\s*(?:const )?val (`?\w+`?) = (.+)$/);
  const brand: Record<string, string> = {};
  const brandBlock = /object CroftBrand \{([^}]*)\}/.exec(text);
  for (const m of (brandBlock?.[1] ?? '').matchAll(/val (\w+) = Color\(0xFF([0-9A-F]{6})\)/g)) {
    brand[m[1] as string] = `#${m[2]}`;
  }
  return { schemes, leaves, brand };
}

export function parseSwift(text: string): Parsed {
  const schemes: ParsedSchemes = new Map();
  const re =
    /CroftSchemeKey\(preset: \.(\w+), mode: \.(\w+), contrast: \.(\w+)\): CroftColorRoles\(([^)]*)\)/g;
  for (const m of text.matchAll(re)) {
    const roles: Record<string, string> = {};
    for (const r of (m[4] as string).matchAll(/(\w+): 0x([0-9A-F]{6}),?/g)) roles[r[1] as string] = `#${r[2]}`;
    schemes.set(`${norm(m[1] as string)}/${modeId(m[2] as string)}/${norm(m[3] as string)}`, roles);
  }
  const leaves = parseNestedTokens(text, 'enum', /^\s*static let (`?\w+`?): [\w[\]]+ = (.+)$/);
  const brand: Record<string, string> = {};
  const brandBlock = /enum CroftBrand \{([^}]*)\}/.exec(text);
  for (const m of (brandBlock?.[1] ?? '').matchAll(/static let (\w+) = Color\(hex: 0x([0-9A-F]{6})\)/g)) {
    brand[m[1] as string] = `#${m[2]}`;
  }
  return { schemes, leaves, brand };
}

export function kebabForTest(word: string): string {
  return word
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1-$2')
    .toLowerCase();
}

export interface ParsedCss {
  schemes: ParsedSchemes;
  vars: Map<string, string>;
  blocks: string[];
}

export function parseCss(text: string): ParsedCss {
  const schemes: ParsedSchemes = new Map();
  const vars = new Map<string, string>();
  const blocks: string[] = [];
  for (const m of text.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const selector = (m[1] as string).replace(/\/\*.*?\*\//gs, '').trim();
    const body = m[2] as string;
    const decls = [...body.matchAll(/(--[\w-]+):\s*([^;]+);/g)];
    const theme = /\[data-croft-theme="(\w+)"\]\[data-croft-mode="([\w-]+)"\](?:\[data-croft-contrast="(\w+)"\])?/.exec(
      selector,
    );
    if (theme) {
      blocks.push(selector);
      const roles: Record<string, string> = {};
      for (const d of decls) {
        const name = d[1] as string;
        if (name.startsWith('--croft-color-')) roles[name] = (d[2] as string).trim();
      }
      schemes.set(`${theme[1]}/${modeId(theme[2] as string)}/${theme[3] ?? 'standard'}`, roles);
    } else if (selector === ':root') {
      for (const d of decls) vars.set(d[1] as string, (d[2] as string).trim());
    }
  }
  return { schemes, vars, blocks };
}

/** CSS role variable name for a role, derived independently of the generator. */
export function cssRoleName(role: string): string {
  return `--croft-color-${kebabForTest(role)}`;
}

export async function importGeneratedTs(dir: string): Promise<Record<string, any>> {
  return (await import(pathToFileURL(join(dir, OUTPUT_FILES.ts)).href)) as Record<string, any>;
}

export function readOutput(dir: string, which: keyof typeof OUTPUT_FILES): string {
  return readFileSync(join(dir, OUTPUT_FILES[which]), 'utf8');
}
