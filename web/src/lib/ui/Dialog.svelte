<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';

  /** A modal dialog (the platform's `<dialog>`: focus is trapped and Escape closes it). */
  let { title, onclose, children }: { title: string; onclose: () => void; children: Snippet } = $props();
  let dialog: HTMLDialogElement | undefined = $state();

  onMount(() => {
    dialog?.showModal();
  });
</script>

<dialog bind:this={dialog} aria-label={title} onclose={onclose} onclick={(e) => e.target === dialog && dialog?.close()}>
  <div class="inner">
    <h2>{title}</h2>
    {@render children()}
    <button class="button text" type="button" onclick={() => dialog?.close()}>Close</button>
  </div>
</dialog>

<style>
  dialog {
    max-width: min(560px, 100vw - 32px);
    padding: 0;
    border: none;
    border-radius: calc(28px * var(--round));
    background: var(--croft-color-surface-container-high);
    color: var(--croft-color-on-surface);
  }

  dialog::backdrop {
    background: color-mix(in srgb, var(--croft-color-scrim) 32%, transparent);
  }

  .inner {
    display: grid;
    gap: 12px;
    padding: 24px;
  }

  h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 500;
  }

  .button {
    justify-self: end;
  }
</style>
