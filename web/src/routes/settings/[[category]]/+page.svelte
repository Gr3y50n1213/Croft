<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { api, type Me } from '#lib/client.ts';
  import Icon, { type IconName } from '#lib/ui/Icon.svelte';
  import AppearancePane from '#lib/settings/AppearancePane.svelte';
  import SwipePane from '#lib/settings/SwipePane.svelte';
  import SecurityPane from '#lib/settings/SecurityPane.svelte';
  import AccountsPane from '#lib/settings/AccountsPane.svelte';
  import AboutPane from '#lib/settings/AboutPane.svelte';
  import NotificationsPane from '#lib/settings/NotificationsPane.svelte';

  /** W04's categories, in its order (more arrive with the features behind them). */
  const CATEGORIES: { id: string; name: string; icon: IconName; words: string }[] = [
    { id: 'accounts', name: 'Accounts', icon: 'account', words: 'Your mail accounts' },
    { id: 'notifications', name: 'Notifications', icon: 'bell', words: 'New mail in this browser' },
    { id: 'swipe', name: 'Swipe and hover actions', icon: 'swipe', words: 'Trackpad swipes and the buttons on a row' },
    { id: 'appearance', name: 'Appearance', icon: 'palette', words: 'Colours, density and shape' },
    { id: 'security', name: 'Security', icon: 'lock', words: 'Passkeys and signed-in devices' },
    { id: 'about', name: 'About', icon: 'info', words: 'Version, licence and privacy' }
  ];

  let me: Me | null = $state(null);
  /** The category in the address; none shows the list (and, on wide windows, Appearance beside it). */
  const chosen = $derived(CATEGORIES.find((c) => c.id === page.params.category) ?? null);
  const shown = $derived(chosen ?? CATEGORIES.find((c) => c.id === 'appearance')!);

  onMount(async () => {
    if (!(await api.current())) return goto('/sign-in', { replaceState: true });
    try {
      me = await api.call<Me>('GET', '/api/me');
    } catch {
      return goto('/sign-in', { replaceState: true });
    }
    if (me.recovery) return goto('/passkey', { replaceState: true });
  });
</script>

<svelte:head><title>{chosen ? `${chosen.name} · Settings` : 'Settings'} · Croft</title></svelte:head>

{#if me}
  <div class="w04" data-screen="W04" class:has-choice={chosen !== null}>
    <header class="bar">
      <a class="back" href="/" aria-label="Back to mail"><Icon name="back" /></a>
      <h1>Settings</h1>
    </header>

    <nav class="categories" aria-label="Settings">
      <ul>
        {#each CATEGORIES as c (c.id)}
          <li>
            <a
              class="category"
              class:current={c.id === shown.id}
              href={`/settings/${c.id}`}
              aria-current={c.id === shown.id ? 'page' : undefined}
              data-sveltekit-replacestate={chosen !== null ? '' : undefined}
            >
              <Icon name={c.icon} />
              <span class="text">
                <span class="name">{c.name}</span>
                <span class="words">{c.words}</span>
              </span>
            </a>
          </li>
        {/each}
      </ul>
    </nav>

    <main class="detail">
      <a class="up" href="/settings">
        <Icon name="back" /> All settings
      </a>
      {#if shown.id === 'appearance'}
        <AppearancePane />
      {:else if shown.id === 'swipe'}
        <SwipePane />
      {:else if shown.id === 'security'}
        <SecurityPane />
      {:else if shown.id === 'notifications'}
        <NotificationsPane user={me.id} />
      {:else if shown.id === 'accounts'}
        <AccountsPane />
      {:else}
        <AboutPane />
      {/if}
    </main>
  </div>
{:else}
  <p class="loading" aria-busy="true">Loading…</p>
{/if}

<style>
  .w04 {
    display: grid;
    grid-template-columns: 320px 1fr;
    grid-template-rows: auto 1fr;
    grid-template-areas: 'bar bar' 'list detail';
    min-height: 100dvh;
  }

  .bar {
    grid-area: bar;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  h1 {
    margin: 0;
    font-size: 22px;
  }

  .back,
  .up {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: var(--croft-touch-min-target-dp);
    min-height: var(--croft-touch-min-target-dp);
    justify-content: center;
    border-radius: 50%;
    color: inherit;
  }

  .up {
    display: none;
    justify-content: start;
    margin-bottom: 8px;
    border-radius: 0;
    text-decoration: none;
  }

  .categories {
    grid-area: list;
    padding: 8px;
    border-right: 1px solid var(--croft-color-outline-variant);
    background: var(--croft-color-surface-container-low);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .category {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 64px;
    padding: 8px 16px;
    border-radius: calc(16px * var(--round));
    color: inherit;
    text-decoration: none;
  }

  .category:hover {
    background: var(--croft-color-surface-container-high);
  }

  .category.current {
    background: var(--croft-color-secondary-container);
    color: var(--croft-color-on-secondary-container);
  }

  .text {
    display: grid;
  }

  .name {
    font-weight: 600;
  }

  .words {
    font-size: 14px;
    color: var(--croft-color-on-surface-variant);
  }

  .current .words {
    color: inherit;
  }

  .detail {
    grid-area: detail;
    padding: 24px 32px;
    overflow: auto;
  }

  .loading {
    padding: 32px;
  }

  /* Narrow windows: the list, then the chosen category on its own. */
  @media (max-width: 839px) {
    .w04 {
      grid-template-columns: 1fr;
      grid-template-areas: 'bar' 'list';
    }

    .w04.has-choice {
      grid-template-areas: 'bar' 'detail';
    }

    .w04:not(.has-choice) .detail,
    .w04.has-choice .categories {
      display: none;
    }

    .categories {
      border-right: none;
    }

    .detail {
      padding: 16px;
    }

    .up {
      display: inline-flex;
    }
  }
</style>
