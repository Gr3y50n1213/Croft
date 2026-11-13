// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Recipient fields as people type them (`Ann Example <ann@example.org>, bob@example.org`) to
 * addresses and back. Validity is the core's to judge (`CroftDraft/check`); this only splits.
 */

import type { Address } from '../jmap/client.ts';

/** Splits on commas and semicolons outside quotes and angle brackets. */
export function parseAddresses(text: string): Address[] {
  const out: Address[] = [];
  let cur = '';
  let quoted = false;
  let angled = false;
  const flush = () => {
    const part = cur.trim();
    cur = '';
    if (!part) return;
    const m = /^(.*?)<([^<>]*)>\s*$/.exec(part);
    if (m) {
      const name = m[1]!.trim().replace(/^"(.*)"$/, '$1').trim();
      out.push({ name: name || null, email: m[2]!.trim() });
    } else {
      out.push({ name: null, email: part });
    }
  };
  for (const ch of text) {
    if (ch === '"' && !angled) quoted = !quoted;
    else if (ch === '<' && !quoted) angled = true;
    else if (ch === '>' && !quoted) angled = false;
    if ((ch === ',' || ch === ';') && !quoted && !angled) flush();
    else cur += ch;
  }
  flush();
  return out;
}

/** Addresses back to the text of a field. */
export function formatAddresses(list: Address[]): string {
  return list
    .map((a) => (a.name ? `${/[,;<>"]/.test(a.name) ? `"${a.name.replace(/"/g, '')}"` : a.name} <${a.email}>` : a.email))
    .join(', ');
}
