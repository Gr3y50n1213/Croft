<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import Choice from './Choice.svelte';
  import { settings } from './store.svelte.ts';
  import {
    CONTRASTS,
    DENSITIES,
    LABELS,
    LIST_STYLES,
    MODES,
    MOTIONS,
    PALETTES,
    ROUNDNESSES,
    paletteName,
    seedOf
  } from './appearance.ts';

  const a = $derived(settings.appearance);
  const label = (o: string) => (o === 'system' ? 'Follow system' : (LABELS[o] ?? o));
  const modeLabel = (o: string) => LABELS[o] ?? o;
  const SAMPLE = [
    { from: 'Ailsa (sample)', subject: 'Seed swap on Saturday', preview: 'Bring anything you saved from last year.', unread: true },
    { from: 'Allotment society', subject: 'Water is back on', preview: 'The taps by the gate work again.', unread: false },
    { from: 'Rory (sample)', subject: 'Re: the shed roof', preview: 'I can help on Sunday morning.', unread: false }
  ];
</script>

<section class="pane" aria-labelledby="appearance-title">
  <h2 id="appearance-title">Appearance</h2>

  <div class="preview" role="img" aria-label="Preview of your inbox with the current look">
    <ul class="sample">
      {#each SAMPLE as s (s.subject)}
        <li class="sample-row" class:unread={s.unread}>
          <span class="avatar" aria-hidden="true">{s.from.charAt(0)}</span>
          <span class="text">
            <span class="from">{s.from}</span>
            <span class="subject">{s.subject}</span>
            <span class="preview-line">{s.preview}</span>
          </span>
        </li>
      {/each}
    </ul>
    <span class="fab">Compose</span>
  </div>

  <Choice legend="Mode" name="mode" options={MODES} value={a.mode} label={modeLabel} onchange={(mode) => settings.setAppearance({ mode })} />

  <label class="switch">
    <input
      type="checkbox"
      checked={a.pureBlack}
      disabled={a.mode === 'light'}
      onchange={(e) => settings.setAppearance({ pureBlack: e.currentTarget.checked })}
    />
    <span>Pure black <span class="hint">Dark mode only</span></span>
  </label>

  <fieldset class="palettes">
    <legend>Palette</legend>
    <div class="swatches">
      {#each PALETTES as p (p)}
        <label class="swatch" class:on={p === a.palette} title={paletteName(p)}>
          <input type="radio" name="palette" value={p} checked={p === a.palette} onchange={() => settings.setAppearance({ palette: p })} />
          <span class="dot" style:background={seedOf(p)} aria-hidden="true"></span>
          <span class="name">{paletteName(p)}</span>
        </label>
      {/each}
    </div>
  </fieldset>

  <Choice legend="Contrast" name="contrast" options={CONTRASTS} value={a.contrast} label={modeLabel} onchange={(contrast) => settings.setAppearance({ contrast })} />
  <Choice
    legend="Density"
    name="density"
    options={DENSITIES}
    value={a.density}
    label={modeLabel}
    hint="How tall each row is, and how many preview lines it shows."
    onchange={(density) => settings.setAppearance({ density })}
  />
  <Choice legend="Roundness" name="roundness" options={ROUNDNESSES} value={a.roundness} label={modeLabel} onchange={(roundness) => settings.setAppearance({ roundness })} />
  <Choice legend="List style" name="list" options={LIST_STYLES} value={a.list} label={modeLabel} onchange={(list) => settings.setAppearance({ list })} />
  <Choice
    legend="Animation"
    name="motion"
    options={MOTIONS}
    value={a.motion}
    {label}
    hint="Reduced keeps fades but nothing that moves across the screen."
    onchange={(motion) => settings.setAppearance({ motion })}
  />
  <p class="row-text"><span class="strong">Haptics</span> Off on the web</p>

  <p class="hint">Saved in this browser.</p>
</section>

<style>
  .pane {
    max-width: 720px;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 22px;
  }

  .preview {
    position: relative;
    margin-bottom: 24px;
    padding: 8px 0 56px;
    border: 1px solid var(--croft-color-outline-variant);
    border-radius: calc(16px * var(--round));
    background: var(--croft-color-surface);
    overflow: hidden;
  }

  .sample {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .sample-row {
    display: flex;
    gap: 12px;
    align-items: center;
    min-height: var(--row-min);
    padding: 8px 16px;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  :global([data-croft-list='container']) .sample-row {
    margin: 4px 8px;
    border-bottom: none;
    border-radius: var(--radius);
    background: var(--croft-color-surface-container-low);
  }

  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: var(--croft-density-default-avatar-dp);
    height: var(--croft-density-default-avatar-dp);
    border-radius: 50%;
    background: var(--croft-color-primary-container);
    color: var(--croft-color-on-primary-container);
    font-weight: 600;
  }

  :global([data-croft-density='comfortable']) .avatar {
    width: var(--croft-density-comfortable-avatar-dp);
    height: var(--croft-density-comfortable-avatar-dp);
  }

  :global([data-croft-density='compact']) .avatar {
    width: var(--croft-density-compact-avatar-dp);
    height: var(--croft-density-compact-avatar-dp);
  }

  .text {
    display: grid;
    min-width: 0;
  }

  .from,
  .subject,
  .preview-line {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .unread .from,
  .unread .subject {
    font-weight: 700;
  }

  .preview-line {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }

  :global([data-croft-density='compact']) .preview-line {
    display: none;
  }

  .fab {
    position: absolute;
    right: 16px;
    bottom: 8px;
    padding: 8px 16px;
    border-radius: calc(16px * var(--round));
    background: var(--croft-color-primary-container);
    color: var(--croft-color-on-primary-container);
    font-weight: 600;
  }

  .switch {
    display: flex;
    gap: 12px;
    align-items: center;
    margin: -8px 0 20px;
    min-height: 40px;
  }

  .switch input {
    width: 20px;
    height: 20px;
    accent-color: var(--croft-color-primary);
  }

  .palettes {
    margin: 0 0 20px;
    padding: 0;
    border: none;
  }

  .palettes legend {
    padding: 0;
    margin-bottom: 8px;
    font-weight: 600;
  }

  .swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .swatch {
    position: relative;
    display: grid;
    justify-items: center;
    gap: 4px;
    width: 64px;
    padding: 6px 0;
    border-radius: calc(12px * var(--round));
    cursor: pointer;
    font-size: 13px;
  }

  .swatch.on {
    background: var(--croft-color-secondary-container);
    color: var(--croft-color-on-secondary-container);
    font-weight: 600;
  }

  .swatch:has(input:focus-visible) {
    outline: 3px solid var(--croft-color-primary);
  }

  .swatch input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
    margin: 0;
  }

  .dot {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: 2px solid var(--croft-color-outline-variant);
  }

  .swatch.on .dot {
    border-color: var(--croft-color-on-surface);
  }

  .hint {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }

  .row-text {
    display: flex;
    gap: 12px;
  }

  .strong {
    font-weight: 600;
  }
</style>
