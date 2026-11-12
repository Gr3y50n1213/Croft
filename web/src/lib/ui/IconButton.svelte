<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import Icon, { type IconName } from './Icon.svelte';

  /** An icon-only button: always labelled, and its tooltip names the shortcut (docs/UX.md). */
  let {
    icon,
    label,
    shortcut = '',
    filled = false,
    disabled = false,
    onclick
  }: {
    icon: IconName;
    label: string;
    shortcut?: string;
    filled?: boolean;
    disabled?: boolean;
    onclick: (e: MouseEvent) => void;
  } = $props();
</script>

<button
  class="icon-button"
  type="button"
  aria-label={label}
  title={shortcut ? `${label} (${shortcut})` : label}
  {disabled}
  {onclick}><Icon name={icon} {filled} /></button>

<style>
  .icon-button {
    display: inline-grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--croft-color-on-surface-variant);
    cursor: pointer;
  }

  .icon-button:hover:not(:disabled) {
    background: color-mix(in srgb, var(--croft-color-on-surface) 8%, transparent);
  }

  .icon-button:disabled {
    opacity: 0.38;
    cursor: default;
  }
</style>
