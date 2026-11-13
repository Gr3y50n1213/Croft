// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Schedule send (S25 on the web, T33a): the preset times, a chosen time as the Companion's
 * `UTCDate`, and how a time reads. Pure (the time "now" is passed in), in the browser's own time
 * zone.
 */

/** How far ahead a message may be scheduled: the core's limit (a year and a day). */
export const MAX_AHEAD_MS = 366 * 24 * 60 * 60 * 1000;

export interface Preset {
  readonly label: string;
  readonly at: Date;
}

function at(day: Date, hour: number): Date {
  const d = new Date(day);
  d.setHours(hour, 0, 0, 0);
  return d;
}

function addDays(day: Date, n: number): Date {
  const d = new Date(day);
  d.setDate(d.getDate() + n);
  return d;
}

/**
 * Tomorrow morning (8:00), tomorrow afternoon (13:00) and Monday morning (the next Monday, 8:00;
 * left out when it is tomorrow morning already).
 */
export function presets(now: Date): Preset[] {
  const tomorrow = addDays(now, 1);
  const out: Preset[] = [
    { label: 'Tomorrow morning', at: at(tomorrow, 8) },
    { label: 'Tomorrow afternoon', at: at(tomorrow, 13) }
  ];
  const toMonday = ((8 - now.getDay()) % 7) || 7;
  if (toMonday !== 1) out.push({ label: 'Monday morning', at: at(addDays(now, toMonday), 8) });
  return out;
}

/** A time as the Companion's `UTCDate` (whole seconds, `Z`). */
export function utcDate(d: Date): string {
  return d.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

/** Why a chosen time cannot be used, in plain words; null when it can. */
export function problemWith(chosen: Date, now: Date): string | null {
  if (Number.isNaN(chosen.getTime())) return 'Choose a date and a time.';
  if (chosen.getTime() <= now.getTime() + 60_000) return 'Choose a time at least a minute from now.';
  if (chosen.getTime() > now.getTime() + MAX_AHEAD_MS) return 'Choose a time within a year.';
  return null;
}

/** The value of an `<input type="datetime-local">` (local time, minutes) as a date. */
export function fromLocalInput(value: string): Date {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(value);
  if (!m) return new Date(NaN);
  const [, y, mo, d, h, mi] = m.map(Number) as [number, number, number, number, number, number];
  const date = new Date(y, mo - 1, d, h, mi, 0, 0);
  // A date that does not exist (31 April) rolls over in Date: refuse it instead.
  return date.getMonth() === mo - 1 && date.getDate() === d ? date : new Date(NaN);
}

/** A local time as the input's value (to start the picker one hour ahead). */
export function toLocalInput(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** "Tomorrow, 08:00", "Mon 17 Nov, 08:00", "17 Nov 2027, 08:00": how a scheduled time reads. */
export function describe(when: Date, now: Date): string {
  const time = when.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const day = new Date(when);
  day.setHours(0, 0, 0, 0);
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const days = Math.round((day.getTime() - today.getTime()) / 86_400_000);
  if (days === 0) return `Today, ${time}`;
  if (days === 1) return `Tomorrow, ${time}`;
  const sameYear = when.getFullYear() === now.getFullYear();
  const date = when.toLocaleDateString([], sameYear ? { weekday: 'short', day: 'numeric', month: 'short' } : { day: 'numeric', month: 'short', year: 'numeric' });
  return `${date}, ${time}`;
}
