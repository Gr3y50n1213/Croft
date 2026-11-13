// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The swipe and hover settings (S47 on the web), stored by action id as the Android app stores them
 * (T30b's types). Kept in this browser until the Companion syncs settings.
 */

import {
  actionFromId,
  defaultSwipeSettings,
  sensitivityFromId,
  type SwipeSettings
} from '../swipe/model.ts';

export const SWIPE_KEY = 'croft.swipe';

/** What was stored, made safe: an unknown or unselectable action falls back to the default. */
export function parseSwipe(stored: string | null | undefined): SwipeSettings {
  let v: Record<string, unknown> = {};
  if (stored) {
    try {
      const parsed: unknown = JSON.parse(stored);
      if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) v = parsed as Record<string, unknown>;
    } catch {
      // Unreadable: the defaults.
    }
  }
  const d = defaultSwipeSettings;
  const action = (x: unknown) => {
    const a = actionFromId(typeof x === 'string' ? x : null);
    return a && a.selectable ? a : null;
  };
  return Object.freeze({
    startToEnd: action(v['startToEnd']) ?? d.startToEnd,
    endToStart: action(v['endToStart']) ?? d.endToStart,
    sensitivity: sensitivityFromId(typeof v['sensitivity'] === 'string' ? v['sensitivity'] : null) ?? d.sensitivity
  });
}

export function serializeSwipe(s: SwipeSettings): string {
  return JSON.stringify({ startToEnd: s.startToEnd.id, endToStart: s.endToStart.id, sensitivity: s.sensitivity });
}
