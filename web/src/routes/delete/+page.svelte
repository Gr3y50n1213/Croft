<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { api } from '#lib/client.ts';
  import { rememberReturn } from '#lib/after-sign-in.ts';

  // T34e: the address to give for deleting your Companion data (the phone app opens it, and stores ask for one). It leads to
  // Settings, Security, "Delete my Companion data", through a sign-in when there is none.
  onMount(async () => {
    rememberReturn(sessionStorage, '/settings/security');
    await goto((await api.current()) ? '/settings/security' : '/sign-in', { replaceState: true });
  });
</script>

<svelte:head><title>Delete my Companion data · Croft</title></svelte:head>

<main class="delete">
  <h1>Delete my Companion data</h1>
  <p>Sign in, then open Settings, Security, and choose "Delete my Companion data".</p>
</main>

<style>
  .delete {
    max-width: 560px;
    margin: 64px auto;
    padding: 0 16px;
  }
</style>
