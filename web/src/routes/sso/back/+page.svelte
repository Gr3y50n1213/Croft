<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { api, explain } from '#lib/sign-in.ts';
  import { takeVerifier } from '#lib/pkce.ts';

  let error = $state('');

  onMount(async () => {
    const params = new URLSearchParams(location.search);
    // The one-time code leaves the address bar (and history) at once.
    history.replaceState(null, '', '/sso/back');
    const code = params.get('code');
    const verifier = takeVerifier(sessionStorage);
    if (params.get('error') || !code || !verifier) {
      error = "Single sign-on didn't finish. Go back and try again.";
      return;
    }
    try {
      const answer = await api.raw('POST', '/api/sign-in/oidc/exchange', { code, verifier, web: true });
      api.adopt(answer);
      await goto('/', { replaceState: true });
    } catch (e) {
      error = explain(e);
    }
  });
</script>

<main class="back">
  {#if error}
    <p class="error" role="alert">{error}</p>
    <a class="button" href="/sign-in">Back to sign-in</a>
  {:else}
    <p>Signing you in…</p>
  {/if}
</main>

<style>
  .back {
    min-height: 100vh;
    display: grid;
    place-content: center;
    gap: 16px;
    text-align: center;
  }

  a.button {
    display: inline-grid;
    place-items: center;
    text-decoration: none;
  }
</style>
