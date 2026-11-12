<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  let { message, canUndo, onundo }: { message: string; canUndo: boolean; onundo: () => void } = $props();
</script>

<div class="snackbar" role="status" aria-live="polite">
  <span>{message}</span>
  {#if canUndo}<button type="button" onclick={onundo} title="Undo (z)">Undo</button>{/if}
</div>

<style>
  .snackbar {
    position: fixed;
    left: 50%;
    bottom: 56px;
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: 24px;
    min-height: 48px;
    padding: 0 8px 0 16px;
    border-radius: 8px;
    background: var(--croft-color-inverse-surface);
    color: var(--croft-color-inverse-on-surface);
  }

  button {
    min-height: 40px;
    padding: 0 12px;
    border: none;
    background: none;
    color: var(--croft-color-inverse-primary);
    font-weight: 600;
    cursor: pointer;
  }
</style>
