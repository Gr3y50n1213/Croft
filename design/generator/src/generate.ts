// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Entry point: node src/generate.ts [--tokens path] [--out dir]
// Deterministic: same tokens.json in, byte-identical files out (no timestamps, stable order).

import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { emitCss } from './emit-css.ts';
import { emitKotlin } from './emit-kotlin.ts';
import { emitSwift } from './emit-swift.ts';
import { emitTs } from './emit-ts.ts';
import { buildAllSchemes, buildStatusColors } from './schemes.ts';
import { flatten, genericTree, loadTokens } from './tokens.ts';

export const OUTPUT_FILES = {
  kotlin: 'kotlin/CroftTokens.kt',
  css: 'css/croft-tokens.css',
  swift: 'swift/CroftTokens.swift',
  ts: 'ts/croft-tokens.ts',
} as const;

const here = dirname(fileURLToPath(import.meta.url));
export const DEFAULT_TOKENS = resolve(here, '../../tokens.json');
export const DEFAULT_OUT = resolve(here, '../../generated');

/** Returns the generated files as { relative path: contents }. */
export function render(tokensPath: string): Record<string, string> {
  const tokens = loadTokens(tokensPath);
  const generic = genericTree(tokens);
  const leaves = flatten(generic);
  const schemes = buildAllSchemes(tokens);
  const status = buildStatusColors(tokens);
  const files = {
    [OUTPUT_FILES.kotlin]: emitKotlin(tokens, generic, schemes, status),
    [OUTPUT_FILES.css]: emitCss(tokens, leaves, schemes, status),
    [OUTPUT_FILES.swift]: emitSwift(tokens, generic, schemes, status),
    [OUTPUT_FILES.ts]: emitTs(tokens, generic, schemes, status),
  };
  return Object.fromEntries(Object.entries(files).map(([k, v]) => [k, v.endsWith('\n') ? v : `${v}\n`]));
}

export function generate(tokensPath: string, outDir: string): string[] {
  const written: string[] = [];
  for (const [rel, text] of Object.entries(render(tokensPath))) {
    const target = join(outDir, rel);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, text);
    written.push(target);
  }
  return written;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const flag = (name: string, fallback: string): string => {
    const i = args.indexOf(name);
    return i >= 0 && args[i + 1] ? resolve(args[i + 1] as string) : fallback;
  };
  for (const file of generate(flag('--tokens', DEFAULT_TOKENS), flag('--out', DEFAULT_OUT))) {
    console.log(`wrote ${file}`);
  }
}
