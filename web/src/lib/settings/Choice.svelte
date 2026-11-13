<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts" generics="T extends string">
  /** One setting as a row of radio buttons (a segmented control); arrow keys move between them. */
  let {
    legend,
    name,
    options,
    value,
    label,
    hint = '',
    onchange
  }: {
    legend: string;
    name: string;
    options: readonly T[];
    value: T;
    label: (o: T) => string;
    hint?: string;
    onchange: (o: T) => void;
  } = $props();
</script>

<fieldset class="choice" data-setting={name}>
  <legend>{legend}</legend>
  <div class="segments">
    {#each options as o (o)}
      <label class:on={o === value}>
        <input type="radio" {name} value={o} checked={o === value} onchange={() => onchange(o)} />
        {label(o)}
      </label>
    {/each}
  </div>
  {#if hint}<p class="hint">{hint}</p>{/if}
</fieldset>

<style>
  .choice {
    margin: 0 0 20px;
    padding: 0;
    border: none;
  }

  legend {
    padding: 0;
    margin-bottom: 8px;
    font-weight: 600;
  }

  .segments {
    display: inline-flex;
    flex-wrap: wrap;
    border: 1px solid var(--croft-color-outline);
    border-radius: calc(20px * var(--round));
    overflow: hidden;
  }

  label {
    position: relative;
    display: inline-flex;
    align-items: center;
    min-height: 40px;
    padding: 0 16px;
    cursor: pointer;
  }

  label + label {
    border-inline-start: 1px solid var(--croft-color-outline);
  }

  label.on {
    background: var(--croft-color-secondary-container);
    color: var(--croft-color-on-secondary-container);
    font-weight: 600;
  }

  label:has(input:focus-visible) {
    outline: 3px solid var(--croft-color-primary);
    outline-offset: -3px;
  }

  input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
    margin: 0;
  }

  .hint {
    margin: 6px 0 0;
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }
</style>
