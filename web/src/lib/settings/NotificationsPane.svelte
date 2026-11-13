<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../client.ts';
  import { explain } from '../api.ts';
  import { disablePush, enablePush, pushState, type PushState } from '../pwa/push.ts';

  let status: PushState = $state('off');
  let busy = $state(false);
  let error = $state('');

  onMount(() => {
    status = pushState();
  });

  async function toggle(on: boolean) {
    error = '';
    busy = true;
    try {
      status = on ? await enablePush(api) : await disablePush(api);
    } catch (e) {
      error = explain(e);
      status = pushState();
    } finally {
      busy = false;
    }
  }
</script>

<section class="pane" aria-labelledby="notifications-title">
  <h2 id="notifications-title">Notifications</h2>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if status === 'unsupported'}
    <p>This browser can't receive notifications from Croft. Croft checks for new mail every minute while it is open.</p>
  {:else}
    <label class="switch">
      <input
        type="checkbox"
        checked={status === 'on'}
        disabled={busy || status === 'blocked'}
        onchange={(e) => toggle(e.currentTarget.checked)}
      />
      <span>
        <span class="name">New mail in this browser</span>
        <span class="hint">
          A notice says "New mail", nothing more: your Companion sends no sender, subject or text through the browser's
          push service. With Croft open, the list just refreshes.
        </span>
      </span>
    </label>
    {#if status === 'blocked'}
      <p class="hint" role="status">Notifications are blocked for this site. Allow them in your browser's site settings, then come back here.</p>
    {/if}
    <p class="hint">Signing out of this browser turns them off; they come back when you sign in again here.</p>
  {/if}
</section>

<style>
  .pane {
    max-width: 720px;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 22px;
  }

  .switch {
    display: flex;
    gap: 12px;
    align-items: start;
    margin-bottom: 16px;
  }

  .switch input {
    margin-top: 4px;
    width: 20px;
    height: 20px;
    accent-color: var(--croft-color-primary);
  }

  .switch > span {
    display: grid;
  }

  .name {
    font-weight: 600;
  }

  .hint {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }
</style>
