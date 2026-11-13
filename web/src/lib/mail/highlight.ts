// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import type { Range } from '../jmap/client.ts';

/** A piece of text, marked when it is a search match. */
export interface Segment {
  text: string;
  mark: boolean;
}

/**
 * Splits `text` at the search's ranges (UTF-16 code units, as the Companion sends them): the text
 * is shown as plain text pieces, never as HTML. Ranges out of order, overlapping or outside the
 * text are clamped or skipped, never trusted.
 */
export function segments(text: string, ranges: readonly Range[] | undefined): Segment[] {
  const out: Segment[] = [];
  let at = 0;
  const sorted = [...(ranges ?? [])]
    .filter((r) => Array.isArray(r) && Number.isInteger(r[0]) && Number.isInteger(r[1]))
    .sort((a, b) => a[0] - b[0]);
  for (const [s, e] of sorted) {
    const start = Math.max(at, Math.min(s, text.length));
    const end = Math.min(Math.max(e, start), text.length);
    if (end <= start) continue;
    if (start > at) out.push({ text: text.slice(at, start), mark: false });
    out.push({ text: text.slice(start, end), mark: true });
    at = end;
  }
  if (at < text.length || out.length === 0) out.push({ text: text.slice(at), mark: false });
  return out;
}
