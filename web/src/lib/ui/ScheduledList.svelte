<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { explainMail, type Jmap, type OutboxItem } from '../jmap/client.ts';
  import type { ComposeState } from '../mail/compose.svelte.ts';
  import { describe } from '../mail/schedule.ts';

  /**
   * Scheduled (T33a): what waits in the account's Outbox in the Companion: scheduled messages, ones
   * in their undo window, and any that failed. Cancel takes one back and reopens its draft.
   */
  let { jmap, account, compose }: { jmap: Jmap; account: string; compose: ComposeState } = $props();

  let items: OutboxItem[] = $state([]);
  let loaded = $state(false);
  let error = $state('');

  async function load() {
    try {
      items = (await jmap.outbox(account)).toSorted((a, b) => a.sendAt.localeCompare(b.sendAt));
      error = '';
    } catch (e) {
      error = explainMail(e);
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    void load();
    compose.onqueued = () => void load();
    // A scheduled message leaves the list when it goes.
    const timer = setInterval(() => void load(), 30_000);
    return () => {
      compose.onqueued = null;
      clearInterval(timer);
    };
  });

  function status(o: OutboxItem): string {
    if (o.state === 'sending') return 'Sending';
    if (o.state === 'failed') return 'Not sent';
    const at = new Date(o.sendAt);
    return at.getTime() - Date.now() > 60_000 ? `Scheduled for ${describe(at, new Date())}` : 'Sending soon';
  }
</script>

<ul class="rows" aria-label="Scheduled" aria-busy={!loaded}>
  {#if error}<li class="error" role="alert">{error}</li>{/if}
  {#each items as o (o.id)}
    <li class="item" data-testid="scheduled">
      <span class="what">
        <span class="subject">{o.subject || '(no subject)'}</span>
        <span class="sub">
          {status(o)} · {o.recipientCount === 1 ? '1 recipient' : `${o.recipientCount} recipients`}
        </span>
      </span>
      {#if o.state === 'queued' && o.draftId}
        <button type="button" class="button text" onclick={() => void compose.takeBack(account, o.id)}>Cancel</button>
      {/if}
    </li>
  {:else}
    {#if loaded && !error}<li class="empty muted">Nothing scheduled.</li>{/if}
  {/each}
</ul>

<style>
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: var(--row-min);
    padding: 8px 16px;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  .what {
    display: grid;
    min-width: 0;
  }

  .subject {
    font-weight: 600;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .sub,
  .muted {
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }

  .empty,
  .error {
    padding: 16px;
  }
</style>
