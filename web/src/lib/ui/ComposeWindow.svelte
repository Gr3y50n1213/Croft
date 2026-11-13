<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import IconButton from './IconButton.svelte';
  import type { ComposeState, ComposeWindow } from '../mail/compose.svelte.ts';
  import { formatAddresses, parseAddresses } from '../mail/addresses.ts';

  /** One compose window (W02): floating bottom right, or docked as a bar when minimised. */
  let { win, compose }: { win: ComposeWindow; compose: ComposeState } = $props();

  // The fields' text starts from the draft once: each window is its own instance (keyed by window).
  let to = $state(untrack(() => formatAddresses(win.fields.to)));
  let cc = $state(untrack(() => formatAddresses(win.fields.cc)));
  let bcc = $state(untrack(() => formatAddresses(win.fields.bcc)));
  let showCopies = $state(untrack(() => win.fields.cc.length + win.fields.bcc.length > 0));
  const title = $derived(win.fields.subject.trim() || 'New message');

  function changed() {
    win.fields.to = parseAddresses(to);
    win.fields.cc = parseAddresses(cc);
    win.fields.bcc = parseAddresses(bcc);
    compose.edited(win.key);
  }

  function keydown(e: KeyboardEvent) {
    // Ctrl or Cmd + Enter sends (as on Android with a keyboard); Escape minimises.
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      changed();
      void compose.send(win.key);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      compose.toggle(win.key);
    }
  }
</script>

{#if win.minimized}
  <div class="docked" role="group" aria-label={`Draft: ${title}`}>
    <button type="button" class="restore" onclick={() => compose.toggle(win.key)}>{title}</button>
    <IconButton icon="close" label="Close" onclick={() => void compose.close(win.key)} />
  </div>
{:else}
  <div class="window" role="dialog" aria-label={title} tabindex="-1" onkeydown={keydown}>
    <div class="head">
      <h2>{title}</h2>
      <IconButton icon="back" label="Minimise" shortcut="Esc" onclick={() => compose.toggle(win.key)} />
      <IconButton icon="close" label="Close" onclick={() => void compose.close(win.key)} />
    </div>
    <label class="row">
      <span>To</span>
      <input autocomplete="off" bind:value={to} oninput={changed} />
      {#if !showCopies}<button type="button" class="link" onclick={() => (showCopies = true)}>Cc Bcc</button>{/if}
    </label>
    {#if showCopies}
      <label class="row"><span>Cc</span><input autocomplete="off" bind:value={cc} oninput={changed} /></label>
      <label class="row"><span>Bcc</span><input autocomplete="off" bind:value={bcc} oninput={changed} /></label>
    {/if}
    <label class="row">
      <span class="sr-only">Subject</span>
      <input placeholder="Subject" bind:value={win.fields.subject} oninput={changed} />
    </label>
    <label class="body">
      <span class="sr-only">Message</span>
      <textarea bind:value={win.fields.body} oninput={changed}></textarea>
    </label>
    <div class="foot">
      <button type="button" class="button" disabled={win.sending} onclick={() => (changed(), void compose.send(win.key))}>Send</button>
      <span class="status" role="status">{win.status}</span>
      <IconButton icon="delete" label="Discard draft" onclick={() => void compose.discard(win.key)} />
    </div>
  </div>
{/if}

<style>
  .window {
    display: flex;
    flex-direction: column;
    width: min(560px, calc(100vw - 32px));
    height: min(600px, calc(100vh - 120px));
    border-radius: calc(16px * var(--round)) calc(16px * var(--round)) 0 0;
    background: var(--croft-color-surface-container-lowest);
    box-shadow: 0 8px 24px color-mix(in srgb, var(--croft-color-scrim) 24%, transparent);
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px 4px 16px;
    background: var(--croft-color-primary);
    color: var(--croft-color-on-primary);
  }

  .head :global(.icon-button) {
    color: var(--croft-color-on-primary);
  }

  h2 {
    flex: 1;
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  .row span {
    color: var(--croft-color-on-surface-variant);
    min-width: 32px;
  }

  input {
    flex: 1;
    min-height: 44px;
    border: none;
    background: transparent;
  }

  .body {
    flex: 1;
    display: flex;
  }

  textarea {
    flex: 1;
    padding: 12px 16px;
    border: none;
    resize: none;
    background: transparent;
    font: inherit;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
  }

  .status {
    flex: 1;
    font-size: 13px;
    color: var(--croft-color-on-surface-variant);
  }

  .docked {
    display: flex;
    align-items: center;
    width: 280px;
    border-radius: calc(12px * var(--round)) calc(12px * var(--round)) 0 0;
    background: var(--croft-color-primary);
    color: var(--croft-color-on-primary);
  }

  .docked :global(.icon-button) {
    color: var(--croft-color-on-primary);
  }

  .restore {
    flex: 1;
    min-height: 44px;
    padding: 0 16px;
    border: none;
    background: none;
    color: inherit;
    text-align: start;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
  }

  .link {
    border: none;
    background: none;
    color: var(--croft-color-primary);
    cursor: pointer;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
