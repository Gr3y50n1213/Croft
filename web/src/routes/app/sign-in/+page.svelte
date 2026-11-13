<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { api } from '#lib/client.ts';
  import { explain } from '#lib/api.ts';
  import { assertionJson, requestOptions } from '#lib/passkey.ts';
  import { startUrl } from '#lib/pkce.ts';

  /**
   * Connect the Croft app (T34a): the Croft app on a phone opens this page in the browser with its
   * PKCE challenge and its redirect. The person signs in here; the page sends the browser to the
   * app with a one-time code (never a session in this browser). The Companion checks that the
   * redirect is the Croft app's own; the page only refuses what is plainly not a request from it.
   */
  const challenge = page.url.searchParams.get('app_challenge') ?? '';
  const redirect = page.url.searchParams.get('app_redirect') ?? '';
  const wellFormed = /^[A-Za-z0-9_-]{43}$/.test(challenge) && /^[a-z][a-z0-9.]*:\/companion$/.test(redirect);

  let busy = $state(false);
  let error = $state('');
  let done = $state(false);
  let sso: { name: string } | null = $state(null);

  onMount(async () => {
    try {
      const info = await api.raw<{ enabled: boolean; name?: string }>('GET', '/api/oidc');
      if (info.enabled) sso = { name: info.name ?? 'single sign-on' };
    } catch {
      // No single sign-on; the passkey still works.
    }
  });

  async function withPasskey() {
    error = '';
    if (!window.PublicKeyCredential) {
      error = "This browser can't use passkeys. Try another browser.";
      return;
    }
    busy = true;
    try {
      const start = await api.raw<{ ceremony: string; options: { publicKey: Record<string, unknown> } }>(
        'POST',
        '/api/sign-in/passkey/start',
        {}
      );
      const cred = (await navigator.credentials.get({
        publicKey: requestOptions(start.options.publicKey)
      })) as PublicKeyCredential | null;
      if (!cred) throw new DOMException('cancelled', 'NotAllowedError');
      const answer = await api.raw<{ redirect: string }>('POST', '/api/sign-in/passkey/finish', {
        ceremony: start.ceremony,
        credential: assertionJson(cred),
        app: { challenge, redirect }
      });
      done = true;
      // Back to the app, with its one-time code.
      location.assign(answer.redirect);
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Connect the Croft app</title></svelte:head>

<main class="sign-in">
  <section class="card" aria-labelledby="title">
    <h1 id="title">Connect the Croft app</h1>
    {#if !wellFormed}
      <p>This page is for the Croft app on your phone. Open it from the app: Settings, Companion.</p>
    {:else if done}
      <p role="status">Signed in. Go back to the Croft app to finish.</p>
    {:else}
      <p>
        Sign in to connect the Croft app on your phone to this Companion. It gets its own sign-in, which you can end from
        Settings, Security on any device.
      </p>
      <p class="warn">Only continue if you started this from the Croft app just now.</p>
      <button class="button" type="button" disabled={busy} onclick={withPasskey}>Sign in with a passkey</button>
      {#if sso}
        <a class="button tonal" href={startUrl(challenge, redirect)}>Sign in with {sso.name}</a>
      {/if}
    {/if}
    <p class="error" role="alert" aria-live="assertive">{error}</p>
  </section>
</main>

<style>
  .sign-in {
    min-height: 100vh;
    display: grid;
    place-items: center;
    padding: var(--gap);
    background: var(--croft-color-surface-container-low);
  }

  .card {
    width: min(100%, 420px);
    display: grid;
    gap: 12px;
    padding: 32px;
    border-radius: calc(28px * var(--round));
    background: var(--croft-color-surface-container-lowest);
    color: var(--croft-color-on-surface);
  }

  h1 {
    margin: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 500;
  }

  p {
    margin: 0;
    color: var(--croft-color-on-surface-variant);
  }

  .warn {
    color: var(--croft-color-on-surface);
    font-weight: 600;
  }

  a.button {
    display: grid;
    place-items: center;
    text-decoration: none;
  }
</style>
