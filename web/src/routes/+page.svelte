<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { api, type Me } from '#lib/client.ts';
  import { RENEW_BEFORE_MS } from '#lib/api.ts';

  let me: Me | null = $state(null);

  onMount(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;

    // Renews the access token a minute before it expires; a refused renewal means sign-in again.
    const schedule = () => {
      clearTimeout(timer);
      const s = api.session;
      if (!s || stopped) return;
      const wait = Math.max(5_000, s.expiresMs - RENEW_BEFORE_MS - Date.now());
      timer = setTimeout(async () => {
        if (!(await api.renew())) return void goto('/sign-in', { replaceState: true });
        schedule();
      }, wait);
    };
    // A tab that slept past its renewal checks again when it is shown.
    const woke = async () => {
      if (document.visibilityState !== 'visible') return;
      if (!(await api.current())) return void goto('/sign-in', { replaceState: true });
      schedule();
    };

    (async () => {
      if (!(await api.current())) return goto('/sign-in', { replaceState: true });
      try {
        me = await api.call<Me>('GET', '/api/me');
      } catch {
        return goto('/sign-in', { replaceState: true });
      }
      if (me.recovery) return goto('/passkey', { replaceState: true });
      schedule();
    })();
    document.addEventListener('visibilitychange', woke);
    return () => {
      stopped = true;
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', woke);
    };
  });

  async function signOut() {
    await api.signOut().catch(() => {});
    await goto('/sign-in', { replaceState: true });
  }
</script>

{#if me}
  <div class="w01" data-screen="W01">
    <header class="bar">
      <span class="brand">Croft</span>
      <span class="who" data-testid="user">{me.name}</span>
      <button class="button text" type="button" onclick={signOut}>Sign out</button>
    </header>
    <nav class="sidebar" aria-label="Folders">
      <p class="muted">Your folders appear here.</p>
    </nav>
    <section class="list" aria-label="Messages">
      <p class="muted">Your mail will be listed here.</p>
    </section>
    <section class="reading" aria-label="Reading pane">
      <p class="muted">Pick a conversation to read it.</p>
    </section>
  </div>
{:else}
  <p class="loading" aria-busy="true">Loading…</p>
{/if}

<style>
  .w01 {
    min-height: 100vh;
    display: grid;
    grid-template-columns: 1fr;
    grid-template-rows: auto 1fr;
    grid-template-areas: 'bar' 'list';
    background: var(--croft-color-surface);
  }

  .bar {
    grid-area: bar;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 8px 16px;
    background: var(--croft-color-surface-container);
  }

  .brand {
    font-weight: 700;
    color: var(--croft-color-primary);
  }

  .who {
    margin-left: auto;
    color: var(--croft-color-on-surface-variant);
  }

  .sidebar,
  .reading {
    display: none;
  }

  .list {
    grid-area: list;
    padding: 16px;
  }

  .muted {
    color: var(--croft-color-on-surface-variant);
  }

  .loading {
    padding: 32px;
    text-align: center;
  }

  /* Expanded width (840 px and up): sidebar, list and reading pane (W01). */
  @media (min-width: 840px) {
    .w01 {
      grid-template-columns: 256px minmax(320px, 2fr) 3fr;
      grid-template-areas: 'bar bar bar' 'sidebar list reading';
    }

    .sidebar {
      display: block;
      grid-area: sidebar;
      padding: 16px;
      background: var(--croft-color-surface-container-low);
    }

    .reading {
      display: block;
      grid-area: reading;
      padding: 16px;
      border-left: 1px solid var(--croft-color-outline-variant);
    }
  }
</style>
