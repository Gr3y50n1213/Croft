// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * W01's keyboard shortcuts (docs/UX.md, "Keyboard, switch and pointer"; Gmail's by default): keys
 * to commands, including the two-key `g` sequences. Pure: the time is passed in. Never while typing
 * in a field, and never with Ctrl, Alt or Meta (the browser's own shortcuts).
 */

export type Command =
  | 'next'
  | 'previous'
  | 'open'
  | 'back'
  | 'select'
  | 'archive'
  | 'delete'
  | 'mark-read'
  | 'mark-unread'
  | 'star'
  | 'reply'
  | 'reply-all'
  | 'forward'
  | 'compose'
  | 'search'
  | 'undo'
  | 'go-inbox'
  | 'go-drafts'
  | 'go-sent'
  | 'help';

export interface Key {
  key: string;
  shift: boolean;
  /** Ctrl, Alt or Meta held. */
  modified: boolean;
  /** The focus is in a text field. */
  typing: boolean;
  timeMs: number;
}

/** The table shown by `?` (and in the hint bar). */
export const SHORTCUTS: readonly [string, string][] = [
  ['j / k', 'Next and previous conversation'],
  ['Enter or o', 'Open the selected conversation'],
  ['u', 'Back to the list'],
  ['x', 'Select the conversation'],
  ['e', 'Archive'],
  ['#', 'Delete'],
  ['Shift + i / Shift + u', 'Mark read / mark unread'],
  ['s', 'Star'],
  ['r, a, f', 'Reply, Reply all, Forward'],
  ['c', 'Compose'],
  ['/', 'Search'],
  ['z', 'Undo the last action'],
  ['g then i, d, s', 'Go to Inbox, Drafts, Sent'],
  ['?', 'Show the shortcuts']
];

/** A second key after `g` must come within this time. */
export const SEQUENCE_MS = 1500;

const SINGLE: Record<string, Command> = {
  j: 'next',
  k: 'previous',
  Enter: 'open',
  o: 'open',
  u: 'back',
  x: 'select',
  e: 'archive',
  '#': 'delete',
  s: 'star',
  r: 'reply',
  a: 'reply-all',
  f: 'forward',
  c: 'compose',
  '/': 'search',
  z: 'undo',
  '?': 'help'
};

const AFTER_G: Record<string, Command> = { i: 'go-inbox', d: 'go-drafts', s: 'go-sent' };

/** Turns keys into commands; keeps the pending `g`. */
export class Shortcuts {
  #gAt: number | null = null;

  /** The command for a key press, or null (not a shortcut, or the first key of a sequence). */
  press(k: Key): Command | null {
    if (k.typing || k.modified) {
      this.#gAt = null;
      return null;
    }
    if (this.#gAt !== null && k.timeMs - this.#gAt <= SEQUENCE_MS) {
      this.#gAt = null;
      return AFTER_G[k.key.toLowerCase()] ?? null;
    }
    this.#gAt = null;
    if (k.shift && (k.key === 'I' || k.key === 'i')) return 'mark-read';
    if (k.shift && (k.key === 'U' || k.key === 'u')) return 'mark-unread';
    if (k.key === 'g' && !k.shift) {
      this.#gAt = k.timeMs;
      return null;
    }
    // `#` and `?` need Shift on most layouts: the key itself says which.
    if (k.shift && k.key !== '#' && k.key !== '?') return null;
    return SINGLE[k.key] ?? null;
  }
}
