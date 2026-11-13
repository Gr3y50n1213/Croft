<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { api } from '#lib/client.ts';
  import { explain } from '#lib/api.ts';
  import { assertionJson, requestOptions } from '#lib/passkey.ts';
  import { challengeOf, keepVerifier, newVerifier, startUrl } from '#lib/pkce.ts';
  import { hasReturn, takeReturn } from '#lib/after-sign-in.ts';

  let busy = $state(false);
  let error = $state('');
  let sso: { name: string } | null = $state(null);
  let recovering = $state(false);
  let user = $state('');
  let totp = $state('');
  let code = $state('');

  onMount(async () => {
    // Still signed in (the cookie)? Then straight to the app.
    if (await api.current()) {
      await goto('/', { replaceState: true });
      return;
    }
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
      const answer = await api.raw('POST', '/api/sign-in/passkey/finish', {
        ceremony: start.ceremony,
        credential: assertionJson(cred),
        device: deviceName(),
        web: true
      });
      api.adopt(answer);
      await goto(takeReturn(sessionStorage), { replaceState: true });
    } catch (e) {
      error = explain(e);
    } finally {
      busy = false;
    }
  }

  async function withSso() {
    const verifier = newVerifier();
    keepVerifier(sessionStorage, verifier);
    const back = `${location.origin}/sso/back`;
    location.assign(startUrl(await challengeOf(verifier), back, hasReturn(sessionStorage)));
  }

  async function withRecovery(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    busy = true;
    try {
      const answer = await api.raw('POST', '/api/sign-in/recovery', {
        user: user.trim(),
        totp: totp.trim(),
        recovery_code: code.trim(),
        device: deviceName(),
        web: true
      });
      api.adopt(answer);
      totp = '';
      code = '';
      await goto('/passkey', { replaceState: true });
    } catch (err) {
      error = explain(err);
    } finally {
      busy = false;
    }
  }

  function deviceName(): string {
    const ua = navigator.userAgent;
    const browser = /Firefox\//.test(ua) ? 'Firefox' : /Edg\//.test(ua) ? 'Edge' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Browser';
    return `${browser} (web)`;
  }
</script>

<main class="sign-in">
  <section class="card" aria-labelledby="title">
    <h1 id="title">Sign in to Croft</h1>
    {#if !recovering}
      <p>Use the passkey you made when you joined this Companion.</p>
      <button class="button" type="button" disabled={busy} onclick={withPasskey}>Sign in with a passkey</button>
      {#if sso}
        <button class="button tonal" type="button" disabled={busy} onclick={withSso}>Sign in with {sso.name}</button>
      {/if}
      <button class="button text" type="button" onclick={() => ((recovering = true), (error = ''))}>
        Lost your passkey?
      </button>
    {:else}
      <p>Sign in with your authenticator code and one of your recovery codes, then make a new passkey.</p>
      <form onsubmit={withRecovery}>
        <label class="field">User name <input autocomplete="username" bind:value={user} required /></label>
        <label class="field">
          Authenticator code
          <input autocomplete="one-time-code" inputmode="numeric" bind:value={totp} required />
        </label>
        <label class="field">Recovery code <input autocomplete="off" bind:value={code} required /></label>
        <button class="button" type="submit" disabled={busy}>Sign in</button>
        <button class="button text" type="button" onclick={() => ((recovering = false), (error = ''))}>Back</button>
      </form>
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

  form {
    display: grid;
    gap: 12px;
  }
</style>
