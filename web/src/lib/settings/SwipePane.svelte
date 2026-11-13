<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import Choice from './Choice.svelte';
  import { settings } from './store.svelte.ts';
  import {
    SwipePresets,
    actionFromId,
    defaultSwipeSettings,
    matchingPreset,
    swipePicker,
    swipeSensitivities,
    withAction,
    type SwipeActionId,
    type SwipeDirection
  } from '../swipe/model.ts';

  const s = $derived(settings.swipe);
  const preset = $derived(matchingPreset(s));

  const ACTION_NAMES: Record<SwipeActionId, string> = {
    read: 'Mark read or unread',
    archive: 'Archive',
    delete: 'Delete',
    snooze: 'Snooze',
    move: 'Move to',
    star: 'Star',
    spam: 'Spam',
    none: 'None'
  };
  const PRESETS = [
    { p: SwipePresets.TRIAGE, name: 'Triage', words: 'Right marks read, left deletes' },
    { p: SwipePresets.GMAIL_STYLE, name: 'Gmail style', words: 'Both archive' },
    { p: SwipePresets.TIDY, name: 'Tidy', words: 'Right marks read, left archives' }
  ];
  const SENS: Record<string, string> = { low: 'Low', normal: 'Normal', high: 'High' };

  /** In right-to-left layouts the stored start-to-end direction is a swipe to the left. */
  const rtl = typeof document !== 'undefined' && document.dir === 'rtl';
  const directions: { d: SwipeDirection; label: string }[] = [
    { d: 'start-to-end', label: rtl ? 'Swipe left' : 'Swipe right' },
    { d: 'end-to-start', label: rtl ? 'Swipe right' : 'Swipe left' }
  ];

  function choose(d: SwipeDirection, id: string) {
    const a = actionFromId(id);
    if (a) settings.setSwipe(withAction(s, d, a));
  }
</script>

<section class="pane" aria-labelledby="swipe-title">
  <h2 id="swipe-title">Swipe and hover actions</h2>
  <p class="lead">
    Two-finger swipes on a trackpad act on a message in the list, and the first buttons that show when you point at a
    message are the same two actions.
  </p>

  <fieldset class="presets">
    <legend>Presets</legend>
    {#each PRESETS as x (x.p.id)}
      <label class="preset" class:on={preset?.id === x.p.id}>
        <input
          type="radio"
          name="preset"
          checked={preset?.id === x.p.id}
          onchange={() => settings.setSwipe({ ...s, startToEnd: x.p.startToEnd, endToStart: x.p.endToStart })}
        />
        <span class="name">{x.name}</span>
        <span class="words">{x.words}</span>
      </label>
    {/each}
  </fieldset>

  {#each directions as dir (dir.d)}
    <label class="field-row">
      <span class="name">{dir.label}</span>
      <select
        value={dir.d === 'start-to-end' ? s.startToEnd.id : s.endToStart.id}
        onchange={(e) => choose(dir.d, e.currentTarget.value)}
      >
        {#each swipePicker as a (a.id)}
          <option value={a.id}>{ACTION_NAMES[a.id]}</option>
        {/each}
      </select>
    </label>
  {/each}

  <Choice
    legend="Sensitivity"
    name="sensitivity"
    options={swipeSensitivities}
    value={s.sensitivity}
    label={(o) => SENS[o] ?? o}
    hint="How far a swipe must go before it acts. High acts on a shorter swipe."
    onchange={(sensitivity) => settings.setSwipe({ ...s, sensitivity })}
  />

  <p class="hint">Deleting forever always asks first.</p>
  <button class="button tonal" type="button" onclick={() => settings.setSwipe(defaultSwipeSettings)}>Reset to default</button>
</section>

<style>
  .pane {
    max-width: 720px;
  }

  h2 {
    margin: 0 0 8px;
    font-size: 22px;
  }

  .lead,
  .hint {
    color: var(--croft-color-on-surface-variant);
  }

  .presets {
    display: grid;
    gap: 8px;
    margin: 16px 0 20px;
    padding: 0;
    border: none;
  }

  .presets legend {
    padding: 0;
    margin-bottom: 8px;
    font-weight: 600;
  }

  .preset {
    position: relative;
    display: grid;
    padding: 10px 16px;
    border: 1px solid var(--croft-color-outline-variant);
    border-radius: calc(12px * var(--round));
    cursor: pointer;
  }

  .preset.on {
    border-color: var(--croft-color-primary);
    background: var(--croft-color-secondary-container);
    color: var(--croft-color-on-secondary-container);
  }

  .preset:has(input:focus-visible) {
    outline: 3px solid var(--croft-color-primary);
  }

  .preset input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
    margin: 0;
  }

  .name {
    font-weight: 600;
  }

  .words {
    font-size: 14px;
  }

  .field-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    max-width: 420px;
    margin-bottom: 16px;
  }

  select {
    min-height: 40px;
    padding: 0 12px;
    border: 1px solid var(--croft-color-outline);
    border-radius: calc(8px * var(--round));
    background: var(--croft-color-surface-container-lowest);
    color: inherit;
    font: inherit;
  }
</style>
