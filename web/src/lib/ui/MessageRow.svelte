<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import IconButton from './IconButton.svelte';
  import type { EmailSummary, SearchHit } from '../jmap/client.ts';
  import { segments } from '../mail/highlight.ts';
  import type { MailAction } from '../mail/actions.ts';
  import { SwipeMachine, slotOf, type SwipeEffect } from '../swipe/machine.ts';
  import type { SwipeActionInfo } from '../swipe/model.ts';
  import { settings as stored } from '../settings/store.svelte.ts';
  import { tuning } from '../swipe/tuning.ts';

  let {
    email,
    focused,
    selected,
    opened,
    can,
    onopen,
    onselect,
    onact,
    hit = null
  }: {
    email: EmailSummary;
    focused: boolean;
    selected: boolean;
    opened: boolean;
    can: (a: MailAction) => boolean;
    onopen: () => void;
    onselect: () => void;
    onact: (a: MailAction) => void;
    /** What a search matched in this row (T33b): marked in the subject; the snippet replaces the preview. */
    hit?: SearchHit | null;
  } = $props();
  // A search's ranges count in the subject it sends with them (T33 review: the stored subject may
  // differ from Email/get's by characters the core removes).
  const subjectText = $derived(hit?.subject ?? email.subject);
  const subjectParts = $derived(segments(subjectText || '(no subject)', subjectText ? hit?.subjectRanges : []));
  const previewParts = $derived(hit?.snippet ? segments(hit.snippet, hit.snippetRanges) : segments(email.preview ?? '', []));

  const seen = $derived(email.keywords['$seen'] === true);
  const flagged = $derived(email.keywords['$flagged'] === true);
  const sender = $derived(email.from?.[0]?.name || email.from?.[0]?.email || '(no sender)');
  const when = $derived(shortDate(email.receivedAt));

  /** The swipe settings (W04's "Swipe and hover actions") as W01 actions. */
  const settings = $derived(stored.swipe);
  function toMail(a: SwipeActionInfo): MailAction | null {
    switch (a.id) {
      case 'read':
        return 'toggle-read';
      case 'delete':
        return 'delete';
      case 'archive':
        return 'archive';
      case 'star':
        return 'toggle-star';
      case 'spam':
        return 'spam';
      default:
        return null;
    }
  }

  /** Hover buttons: the two swipe actions first, then archive, delete and read (docs/ux/swipe.md). */
  const quick = $derived.by(() => {
    const out: MailAction[] = [];
    for (const a of [settings.startToEnd, settings.endToStart]) {
      const m = toMail(a);
      if (m && !out.includes(m)) out.push(m);
    }
    for (const m of ['archive', 'delete', 'toggle-read'] as MailAction[]) if (!out.includes(m)) out.push(m);
    return out.filter((m) => can(m));
  });

  // ---- Trackpad two-finger swipes drive the same machine as touch on Android (T30b). ----
  let row: HTMLElement | undefined = $state();
  let offset = $state(0);
  let armed = $state(false);
  let machine: SwipeMachine | null = null;
  let dx = 0;
  let last = { t: 0, x: 0 };
  let velocity = 0;
  let idle: ReturnType<typeof setTimeout> | undefined;

  function slot(a: SwipeActionInfo) {
    const m = toMail(a);
    return slotOf(a, m !== null && can(m));
  }

  function wheel(e: WheelEvent) {
    if (Math.abs(e.deltaX) <= Math.abs(e.deltaY) || !row) return;
    e.preventDefault();
    const now = e.timeStamp;
    if (!machine) {
      const rtl = getComputedStyle(row).direction === 'rtl';
      machine = new SwipeMachine(tuning, {
        width: row.clientWidth,
        rtl,
        sensitivity: settings.sensitivity,
        startToEnd: slot(settings.startToEnd),
        endToStart: slot(settings.endToStart)
      });
      dx = 0;
      last = { t: now, x: 0 };
    }
    // Content follows the fingers: a swipe to the right scrolls left (negative deltaX).
    dx -= e.deltaX;
    const dt = Math.max(1, now - last.t);
    velocity = ((dx - last.x) / dt) * 1000;
    last = { t: now, x: dx };
    run(machine.send({ type: 'drag', dx, timeMs: now }));
    clearTimeout(idle);
    idle = setTimeout(() => {
      if (machine) run(machine.send({ type: 'release', velocity, timeMs: performance.now() }));
    }, 120);
  }

  function run(effects: readonly SwipeEffect[]) {
    if (!machine) return;
    offset = machine.state.offset;
    armed = machine.state.phase === 'armed';
    for (const fx of effects) {
      if (fx.type === 'commit' || fx.type === 'park') {
        const m = toMail(fx.action);
        if (m) onact(m);
        end();
      } else if (fx.type === 'spring-back') {
        end();
      }
    }
  }

  function end() {
    machine = null;
    offset = 0;
    armed = false;
  }

  function shortDate(iso: string): string {
    const d = new Date(iso);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
    return d.toLocaleDateString();
  }

  const labels: Record<MailAction, [string, string]> = {
    archive: ['Archive', 'e'],
    delete: ['Delete', '#'],
    spam: ['Spam', ''],
    move: ['Move to', ''],
    'toggle-read': ['Mark read or unread', 'Shift+i / Shift+u'],
    'mark-read': ['Mark read', 'Shift+i'],
    'mark-unread': ['Mark unread', 'Shift+u'],
    'toggle-star': ['Star', 's']
  };
  const iconOf = (m: MailAction) =>
    m === 'archive' ? 'archive' : m === 'delete' ? 'delete' : m === 'toggle-star' ? 'star' : m === 'spam' ? 'spam' : seen ? 'unread' : 'read';
</script>

<li
  bind:this={row}
  class="row"
  class:unread={!seen}
  class:focused
  class:opened
  class:armed
  data-id={email.id}
  onwheel={wheel}
  style:transform={offset ? `translateX(${offset}px)` : null}
>
  <input
    class="check"
    type="checkbox"
    checked={selected}
    aria-label={`Select ${email.subject ?? 'this email'}`}
    onclick={(e) => {
      e.stopPropagation();
      onselect();
    }}
  />
  <button class="main" type="button" aria-current={opened ? 'true' : undefined} onclick={onopen}>
    <span class="top">
      <span class="sender">{sender}</span>
      <span class="when">{when}</span>
    </span>
    <span class="subject">{#each subjectParts as p, i (i)}{#if p.mark}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}</span>
    <span class="preview">{#each previewParts as p, i (i)}{#if p.mark}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}</span>
    <span class="marks">
      {#if flagged}<span class="star" title="Starred"><Icon name="star" filled size={16} /></span>{/if}
      {#if !seen}<span class="sr-only">Unread</span>{/if}
    </span>
  </button>
  <span class="quick">
    {#each quick as m (m)}
      <IconButton icon={iconOf(m)} label={labels[m][0]} shortcut={labels[m][1]} onclick={(e) => (e.stopPropagation(), onact(m))} />
    {/each}
  </span>
</li>

<style>
  .row {
    list-style: none;
    position: relative;
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: start;
    gap: 8px;
    padding: 8px 12px;
    min-height: var(--row-min);
    border-bottom: 1px solid var(--croft-color-outline-variant);
    background: var(--croft-color-surface);
    transition: transform var(--croft-motion-spatial-fast-easing-duration) var(--croft-motion-spatial-fast-easing);
  }

  /* Container: rows as cards with a gap; Flat (the plain list): full width with dividers. */
  :global([data-croft-list='container']) .row {
    margin: 4px 8px;
    border-bottom: none;
    border-radius: var(--radius);
    background: var(--croft-color-surface-container-low);
  }

  .row.focused {
    box-shadow: inset 4px 0 0 var(--croft-color-primary);
  }

  .row.opened {
    background: var(--croft-color-secondary-container);
  }

  .row.armed {
    background: var(--croft-color-primary-container);
  }

  .check {
    margin-top: 6px;
    width: 18px;
    height: 18px;
    accent-color: var(--croft-color-primary);
  }

  .main {
    display: grid;
    gap: 2px;
    min-width: 0;
    padding: 0;
    border: none;
    background: none;
    text-align: start;
    cursor: pointer;
  }

  .top {
    display: flex;
    gap: 8px;
  }

  .sender,
  .subject,
  .preview {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  /* Preview lines follow the density: two when comfortable, none when compact. */
  :global([data-croft-density='comfortable']) .preview {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    white-space: normal;
  }

  :global([data-croft-density='compact']) .preview {
    display: none;
  }

  .sender {
    flex: 1;
  }

  .when,
  .preview {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }

  mark {
    border-radius: calc(3px * var(--round));
    background: var(--croft-color-tertiary-container);
    color: var(--croft-color-on-tertiary-container);
  }

  .unread .sender,
  .unread .subject {
    font-weight: 700;
  }

  .marks {
    position: absolute;
    right: 12px;
    bottom: 10px;
    color: var(--croft-status-amber);
  }

  .quick {
    position: absolute;
    right: 8px;
    top: 50%;
    translate: 0 -50%;
    display: none;
    gap: 2px;
    padding: 2px;
    border-radius: calc(24px * var(--round));
    background: var(--croft-color-surface-container-high);
  }

  .row:hover .quick,
  .row:focus-within .quick {
    display: flex;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
