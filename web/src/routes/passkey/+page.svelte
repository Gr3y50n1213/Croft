<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { api, explain } from '#lib/sign-in.ts';
  import { attestationJson, creationOptions } from '#lib/passkey.ts';

  let busy = $state(false);
  let error = $state('');
  let done = $state(false);

  onMount(async () => {
    if (!(await api.current())) await goto('/sign-in', { replaceState: true });
  });

  async function make() {
    error = '';
    busy = true;
    try {
      const start = await api.call<{ ceremony: string; options: { publicKey: Record<string, unknown> } }>(
        'POST',
        '/api/passkeys/start'
      );
      const cred = (await navigator.credentials.create({
        publicKey: creationOptions(start.options.publicKey)
      })) as PublicKeyCredential | null;
      if (!cred) throw new DOMException('cancelled', 'NotAllowedError');
      await api.call('POST', '/api/passkeys/finish', { ceremony: start.ceremony, credential: attestationJson(cred) });
      done = true;
      // A recovery sign-in only lets you add a passkey: sign in again with it.
      await api.signOut();
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }
</script>

<main class="passkey">
  <section class="card">
    {#if done}
      <h1>Passkey made</h1>
      <p>Sign in again with your new passkey.</p>
      <a class="button" href="/sign-in">Sign in</a>
    {:else}
      <h1>Make a new passkey</h1>
      <p>You signed in with a recovery code. Make a passkey on this device to sign in from now on.</p>
      <button class="button" type="button" disabled={busy} onclick={make}>Make a passkey</button>
    {/if}
    <p class="error" role="alert">{error}</p>
  </section>
</main>

<style>
  .passkey {
    min-height: 100vh;
    display: grid;
    place-items: center;
    padding: var(--gap);
  }

  .card {
    width: min(100%, 420px);
    display: grid;
    gap: 12px;
  }

  h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 500;
  }

  a.button {
    display: inline-grid;
    place-items: center;
    text-decoration: none;
  }
</style>
