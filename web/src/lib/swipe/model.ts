// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The swipe settings types (docs/ux/swipe.md), ported one for one from the Android module
 * `android/swipe` (SwipeModel.kt) so both apps keep the same rules (T30b).
 */

/** What a swipe can do; the id is what the settings store. */
export interface SwipeActionInfo {
  readonly id: SwipeActionId;
  /** The row leaves the list (the commit slides it out and collapses its slot). */
  readonly removesRow: boolean;
  /** The action needs more from the person (a sheet), so the card is held while it is open. */
  readonly parks: boolean;
  /** Whether S48 offers it (Snooze waits for its sheet). */
  readonly selectable: boolean;
}

export type SwipeActionId = 'none' | 'read' | 'delete' | 'archive' | 'snooze' | 'move' | 'star' | 'spam';

const a = (id: SwipeActionId, removesRow: boolean, parks: boolean, selectable = true): SwipeActionInfo =>
  Object.freeze({ id, removesRow, parks, selectable });

/** Every action, by its Kotlin name. */
export const SwipeAction = Object.freeze({
  /** The direction does nothing; the row does not move at all. */
  NONE: a('none', false, false),
  /** Mark read, or unread when the row is read. */
  TOGGLE_READ: a('read', false, false),
  /** Move to Trash (in Trash and Spam: delete forever, which parks behind a confirmation). */
  DELETE: a('delete', true, false),
  /** Move to Archive. */
  ARCHIVE: a('archive', true, false),
  /** Opens the snooze sheet (S29). Not offered until Snooze exists. */
  SNOOZE: a('snooze', true, true, false),
  /** Opens the folder picker (S30). */
  MOVE_TO: a('move', true, true),
  /** Star or unstar. */
  TOGGLE_STAR: a('star', false, false),
  /** Move to Spam. */
  SPAM: a('spam', true, false)
});

export type SwipeActionT = SwipeActionInfo;

/** Every action, in declaration order (Kotlin's `entries`). */
export const swipeActions: readonly SwipeActionInfo[] = Object.freeze(Object.values(SwipeAction));

/** The action with this stored id, or null. */
export function actionFromId(id: string | null | undefined): SwipeActionInfo | null {
  return swipeActions.find((x) => x.id === id) ?? null;
}

/** The actions S48 lists, in its order. */
export const swipePicker: readonly SwipeActionInfo[] = Object.freeze(
  [
    SwipeAction.TOGGLE_READ,
    SwipeAction.ARCHIVE,
    SwipeAction.DELETE,
    SwipeAction.SNOOZE,
    SwipeAction.MOVE_TO,
    SwipeAction.TOGGLE_STAR,
    SwipeAction.SPAM,
    SwipeAction.NONE
  ].filter((x) => x.selectable)
);

/** How far a drag must go before the action arms: low 45, normal 35, high 25 percent (S47). */
export type SwipeSensitivity = 'low' | 'normal' | 'high';
export const swipeSensitivities: readonly SwipeSensitivity[] = Object.freeze(['low', 'normal', 'high']);

export function sensitivityFromId(id: string | null | undefined): SwipeSensitivity | null {
  return swipeSensitivities.find((s) => s === id) ?? null;
}

/**
 * A stored direction: from the start edge toward the end edge, or back. In left-to-right layouts
 * `start-to-end` is a drag to the right; in right-to-left a drag to the left, so one stored setting
 * mirrors by itself.
 */
export type SwipeDirection = 'start-to-end' | 'end-to-start';

/** +1 when the direction is a drag to the right on screen, -1 when to the left. */
export function physicalSign(d: SwipeDirection, rtl: boolean): 1 | -1 {
  return (d === 'start-to-end') !== rtl ? 1 : -1;
}

/** The direction of a physical drag by `dx` (positive is to the right); `dx` must not be zero. */
export function directionOf(dx: number, rtl: boolean): SwipeDirection {
  return dx > 0 !== rtl ? 'start-to-end' : 'end-to-start';
}

/** What the person chose in S47. */
export interface SwipeSettings {
  readonly startToEnd: SwipeActionInfo;
  readonly endToStart: SwipeActionInfo;
  readonly sensitivity: SwipeSensitivity;
}

/** Triage: the default. */
export const defaultSwipeSettings: SwipeSettings = Object.freeze({
  startToEnd: SwipeAction.TOGGLE_READ,
  endToStart: SwipeAction.DELETE,
  sensitivity: 'normal'
});

export function actionOf(s: SwipeSettings, d: SwipeDirection): SwipeActionInfo {
  return d === 'start-to-end' ? s.startToEnd : s.endToStart;
}

export function withAction(s: SwipeSettings, d: SwipeDirection, action: SwipeActionInfo): SwipeSettings {
  return d === 'start-to-end' ? { ...s, startToEnd: action } : { ...s, endToStart: action };
}

/** The presets of S47 ("Inbox zero" waits for Snooze). */
export interface SwipePreset {
  readonly id: 'triage' | 'gmail' | 'tidy';
  readonly startToEnd: SwipeActionInfo;
  readonly endToStart: SwipeActionInfo;
}

export const SwipePresets = Object.freeze({
  /** Right Mark read, left Delete. The default. */
  TRIAGE: Object.freeze({ id: 'triage', startToEnd: SwipeAction.TOGGLE_READ, endToStart: SwipeAction.DELETE }),
  /** Both Archive. */
  GMAIL_STYLE: Object.freeze({ id: 'gmail', startToEnd: SwipeAction.ARCHIVE, endToStart: SwipeAction.ARCHIVE }),
  /** Right Mark read, left Archive. */
  TIDY: Object.freeze({ id: 'tidy', startToEnd: SwipeAction.TOGGLE_READ, endToStart: SwipeAction.ARCHIVE })
} satisfies Record<string, SwipePreset>);

/** The preset the settings equal (sensitivity is not part of a preset), or null. */
export function matchingPreset(s: SwipeSettings): SwipePreset | null {
  return (
    Object.values(SwipePresets).find((p) => p.startToEnd === s.startToEnd && p.endToStart === s.endToStart) ?? null
  );
}
