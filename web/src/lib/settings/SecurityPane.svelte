<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../client.ts';
  import { ApiError, explain } from '../api.ts';
  import { makePasskey } from '../passkey.ts';
  import Dialog from '../ui/Dialog.svelte';
  import { goto } from '$app/navigation';
  import { rememberReturn } from '../after-sign-in.ts';
  import { forgetPushAfterDeletion } from '../pwa/push.ts';

  interface Passkey {
    id: number;
    created_ms: number;
    last_used_ms: number | null;
  }
  interface Device {
    id: number;
    device: string | null;
    created_ms: number;
    last_seen_ms: number;
    current: boolean;
  }

  let passkeys: Passkey[] = $state([]);
  let sessions: Device[] = $state([]);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let removing: Passkey | null = $state(null);
  let deleting = $state(false);
  let typed = $state('');
  let deleteError = $state('');
  let needsFreshSignIn = $state(false);
  let myName = $state('');
  /** Adding a passkey needs a sign-in from the last ten minutes too (T34e). */
  let addNeedsFreshSignIn = $state(false);

  const others = $derived(sessions.filter((s) => !s.current));

  async function load() {
    const [p, s] = await Promise.all([
      api.call<{ passkeys: Passkey[] }>('GET', '/api/passkeys'),
      api.call<{ sessions: Device[] }>('GET', '/api/sessions')
    ]);
    passkeys = p.passkeys;
    sessions = [...s.sessions].sort((a, b) => Number(b.current) - Number(a.current) || b.last_seen_ms - a.last_seen_ms);
    loaded = true;
  }

  onMount(() => {
    load().catch((e) => (error = explain(e)));
    api
      .call<{ name: string }>('GET', '/api/me')
      .then((me) => (myName = me.name))
      .catch(() => {});
  });

  /** T34e: "Delete my Companion data". The Companion asks for the name and a sign-in from the last ten minutes. */
  async function deleteEverything() {
    deleteError = '';
    busy = true;
    try {
      await api.call('POST', '/api/me/delete', { confirm: typed.trim() });
      // The session is gone on the Companion; the browser drops its cookie and its push subscription too.
      await forgetPushAfterDeletion().catch(() => {});
      await api.signOut().catch(() => api.forget());
      await goto('/deleted', { replaceState: true });
    } catch (e) {
      if (e instanceof ApiError && e.code === 'fresh_sign_in') {
        needsFreshSignIn = true;
      } else if (e instanceof ApiError && e.code === 'bad_request') {
        deleteError = 'That is not your user name.';
      } else if (e instanceof ApiError && e.code === 'conflict') {
        deleteError = 'You are the only admin of this Companion. Make someone else an admin first.';
      } else {
        deleteError = explain(e);
      }
    } finally {
      busy = false;
    }
  }

  /** Signs this browser out and back in, then returns here (the deletion needs a fresh sign-in). */
  async function signInAgain() {
    rememberReturn(sessionStorage, '/settings/security');
    await api.signOut();
    await goto('/sign-in', { replaceState: true });
  }

  async function run(work: () => Promise<void>, done: string) {
    error = '';
    notice = '';
    busy = true;
    try {
      await work();
      notice = done;
    } catch (e) {
      if (e instanceof ApiError && e.code === 'fresh_sign_in') {
        addNeedsFreshSignIn = true;
        error = 'For your safety, sign in again before adding a passkey.';
      } else {
        error =
          e instanceof ApiError && e.code === 'conflict'
            ? "Your only passkey can't be removed. Add another one first."
            : explain(e);
      }
    } finally {
      busy = false;
      await load().catch(() => {});
    }
  }

  const add = () => run(() => makePasskey(api), 'Passkey added.');

  function remove() {
    const p = removing;
    removing = null;
    if (p) void run(() => api.call('DELETE', `/api/passkeys/${p.id}`), 'Passkey removed.');
  }

  const signOut = (d: Device) => run(() => api.call('DELETE', `/api/sessions/${d.id}`), 'Signed out.');

  const signOutOthers = () =>
    run(async () => {
      for (const d of others) await api.call('DELETE', `/api/sessions/${d.id}`).catch((e) => {
        // Already gone (it expired or another tab ended it) is what was wanted.
        if (!(e instanceof ApiError && e.status === 404)) throw e;
      });
    }, 'Signed out everywhere else.');

  const day = (ms: number) => new Date(ms).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' });
  const when = (ms: number) =>
    new Date(ms).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
</script>

<section class="pane" aria-labelledby="security-title">
  <h2 id="security-title">Security</h2>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}

  <h3>Passkeys</h3>
  <p class="hint">You sign in to your Companion with a passkey. Keep at least two, on different devices, so losing one never locks you out.</p>
  <ul class="list" aria-label="Passkeys" aria-busy={!loaded}>
    {#each passkeys as p, i (p.id)}
      <li class="item" data-testid="passkey">
        <span class="what">
          <span class="name">Passkey {i + 1}</span>
          <span class="sub">Added {day(p.created_ms)}{p.last_used_ms ? `, last used ${day(p.last_used_ms)}` : ''}</span>
        </span>
        <button
          class="button text"
          type="button"
          disabled={busy || passkeys.length < 2}
          title={passkeys.length < 2 ? "Your only passkey can't be removed" : undefined}
          onclick={() => (removing = p)}>Remove</button
        >
      </li>
    {/each}
  </ul>
  <button class="button tonal" type="button" disabled={busy} onclick={add}>Add a passkey on this device</button>
  {#if addNeedsFreshSignIn}
    <button class="button text" type="button" disabled={busy} onclick={signInAgain}>Sign in again</button>
  {/if}

  <h3>Signed-in devices</h3>
  <ul class="list" aria-label="Signed-in devices" aria-busy={!loaded}>
    {#each sessions as d (d.id)}
      <li class="item" data-testid="session">
        <span class="what">
          <span class="name">{d.device || 'Unknown device'}{#if d.current}<span class="this"> · This browser</span>{/if}</span>
          <span class="sub">Signed in {day(d.created_ms)}, last seen {when(d.last_seen_ms)}</span>
        </span>
        {#if !d.current}
          <button class="button text" type="button" disabled={busy} onclick={() => signOut(d)}>Sign out</button>
        {/if}
      </li>
    {/each}
  </ul>
  {#if others.length > 0}
    <button class="button tonal" type="button" disabled={busy} onclick={signOutOthers}>Sign out everywhere else</button>
  {/if}

  <h3 id="delete">Delete my Companion data</h3>
  <p class="hint">
    Deletes your account on this Companion: its copy of your mail, your mail accounts and their passwords, your passkeys, your
    signed-in devices and their notifications. Your mail on your mail servers and on your phone is not touched. This can't be undone.
  </p>
  <button
    class="button danger"
    type="button"
    disabled={busy}
    data-testid="delete-open"
    onclick={() => {
      deleting = true;
      typed = '';
      deleteError = '';
      needsFreshSignIn = false;
    }}>Delete my Companion data</button
  >
</section>

{#if deleting}
  <Dialog title="Delete your Companion data?" onclose={() => (deleting = false)}>
    {#if needsFreshSignIn}
      <p>For your safety, sign in again first. You'll come back here.</p>
      <button type="button" class="button" data-testid="delete-sign-in" onclick={signInAgain}>Sign in again</button>
    {:else}
      <p>Everything of yours on this Companion is deleted at once, for good. Type your user name{myName ? ` (${myName})` : ''} to confirm.</p>
      <label class="confirm">
        <span>User name</span>
        <input type="text" autocomplete="off" spellcheck="false" bind:value={typed} data-testid="delete-name" />
      </label>
      {#if deleteError}<p class="error" role="alert">{deleteError}</p>{/if}
      <button
        type="button"
        class="button danger"
        disabled={busy || typed.trim() === ''}
        data-testid="delete-confirm"
        onclick={deleteEverything}>Delete everything</button
      >
    {/if}
  </Dialog>
{/if}

{#if removing}
  <Dialog title="Remove this passkey?" onclose={() => (removing = null)}>
    <p>The device that holds it can no longer sign in to your Companion.</p>
    <button type="button" class="button danger" onclick={remove}>Remove</button>
  </Dialog>
{/if}

<style>
  .pane {
    max-width: 720px;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 22px;
  }

  h3 {
    margin: 24px 0 4px;
    font-size: 18px;
  }

  .hint,
  .sub {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }

  .list {
    margin: 8px 0 12px;
    padding: 0;
    list-style: none;
  }

  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 56px;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  .what {
    display: grid;
  }

  .name {
    font-weight: 600;
  }

  .this {
    color: var(--croft-color-primary);
  }

  .notice {
    color: var(--croft-color-primary);
  }
</style>
