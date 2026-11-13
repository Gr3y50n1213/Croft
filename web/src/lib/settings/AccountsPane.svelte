<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../client.ts';
  import { Jmap, explainMail, type Account } from '../jmap/client.ts';

  let accounts: Account[] = $state([]);
  let loaded = $state(false);
  let error = $state('');

  onMount(() => {
    new Jmap(api)
      .accounts()
      .then((a) => (accounts = a))
      .catch((e) => (error = explainMail(e)))
      .finally(() => (loaded = true));
  });
</script>

<section class="pane" aria-labelledby="accounts-title">
  <h2 id="accounts-title">Accounts</h2>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <ul class="list" aria-label="Mail accounts" aria-busy={!loaded}>
    {#each accounts as a, i (a.id)}
      <li class="item" data-testid="account">
        <span class="name">{a.name}</span>
        {#if i === 0}<span class="sub">Main account</span>{/if}
      </li>
    {:else}
      {#if loaded && !error}<li class="item sub">No mail accounts yet.</li>{/if}
    {/each}
  </ul>
  <p class="hint">Add or remove mail accounts in the Croft app on your phone, or ask your admin.</p>
</section>

<style>
  .pane {
    max-width: 720px;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 22px;
  }

  .list {
    margin: 0 0 12px;
    padding: 0;
    list-style: none;
  }

  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 56px;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  .name {
    font-weight: 600;
  }

  .hint,
  .sub {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }
</style>
