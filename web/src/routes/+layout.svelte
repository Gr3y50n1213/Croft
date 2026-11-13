<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  // The generated tokens, read in place (design/generated is never copied).
  import '../../../design/generated/css/croft-tokens.css';
  import '../app.css';
  import { onMount } from 'svelte';
  import { settings } from '#lib/settings/store.svelte.ts';
  import { registerWorker } from '#lib/pwa/register.ts';

  let { children } = $props();
  let online = $state(true);

  onMount(() => {
    // W04's look, applied by static/theme.js before the first paint, is followed from here on.
    settings.start();
    // The installable app: its files cached, so it opens offline (T32b).
    void registerWorker();
    online = navigator.onLine;
    const on = () => (online = true);
    const off = () => (online = false);
    window.addEventListener('online', on);
    window.addEventListener('offline', off);
    return () => {
      window.removeEventListener('online', on);
      window.removeEventListener('offline', off);
    };
  });
</script>

{#if !online}
  <p class="offline" role="status">Offline. Croft catches up when the connection is back.</p>
{/if}
{@render children()}

<style>
  .offline {
    position: fixed;
    inset: auto 0 0 0;
    z-index: 30;
    margin: 0;
    padding: 8px 16px;
    background: var(--croft-color-inverse-surface);
    color: var(--croft-color-inverse-on-surface);
    text-align: center;
  }
</style>
