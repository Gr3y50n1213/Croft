<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import Icon, { type IconName } from './Icon.svelte';
  import type { Account, Mailbox } from '../jmap/client.ts';

  let {
    accounts,
    account,
    mailboxes,
    current,
    onaccount,
    onmailbox,
    oncompose
  }: {
    accounts: Account[];
    account: string | null;
    mailboxes: Mailbox[];
    current: string | null;
    onaccount: (id: string) => void;
    onmailbox: (id: string) => void;
    oncompose: () => void;
  } = $props();

  const ICONS: Record<string, IconName> = {
    inbox: 'inbox',
    drafts: 'drafts',
    sent: 'sent',
    outbox: 'outbox',
    archive: 'archive',
    junk: 'spam',
    trash: 'trash'
  };
  const NAMES: Record<string, string> = {
    inbox: 'Inbox',
    drafts: 'Drafts',
    sent: 'Sent',
    outbox: 'Outbox',
    archive: 'Archive',
    junk: 'Spam',
    trash: 'Trash'
  };
</script>

<nav class="sidebar" aria-label="Folders">
  <button class="compose" type="button" title="Compose (c)" onclick={oncompose}>
    <Icon name="compose" /> Compose
  </button>
  <ul class="folders">
    {#each mailboxes as m (m.id)}
      <li>
        <button
          type="button"
          class="folder"
          class:current={m.id === current}
          aria-current={m.id === current ? 'page' : undefined}
          onclick={() => onmailbox(m.id)}
        >
          <Icon name={m.role ? (ICONS[m.role] ?? 'folder') : 'folder'} />
          <span class="name">{m.role ? (NAMES[m.role] ?? m.name) : m.name}</span>
          {#if m.unreadEmails > 0 && m.role !== 'sent' && m.role !== 'trash'}
            <span class="count" aria-label={`${m.unreadEmails} unread`}>{m.unreadEmails}</span>
          {/if}
        </button>
      </li>
    {/each}
  </ul>
  {#if accounts.length > 1}
    <p class="label" id="accounts-label">Accounts</p>
    <ul class="accounts" aria-labelledby="accounts-label">
      {#each accounts as a (a.id)}
        <li>
          <button type="button" class="folder" class:current={a.id === account} onclick={() => onaccount(a.id)}>
            <span class="name">{a.name}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</nav>

<style>
  .sidebar {
    display: grid;
    align-content: start;
    gap: 8px;
    padding: 12px;
    background: var(--croft-color-surface-container-low);
    overflow-y: auto;
  }

  .compose {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 0 20px;
    border: none;
    border-radius: calc(16px * var(--round));
    background: var(--croft-color-primary-container);
    color: var(--croft-color-on-primary-container);
    font-weight: 600;
  }

  .compose:disabled {
    opacity: 0.6;
  }

  .folders,
  .accounts {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .folder {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 44px;
    padding: 0 16px;
    border: none;
    border-radius: calc(22px * var(--round));
    background: transparent;
    text-align: start;
    cursor: pointer;
  }

  .folder:hover {
    background: color-mix(in srgb, var(--croft-color-on-surface) 8%, transparent);
  }

  .folder.current {
    background: var(--croft-color-secondary-container);
    color: var(--croft-color-on-secondary-container);
    font-weight: 600;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count {
    font-size: 13px;
  }

  .label {
    margin: 16px 16px 0;
    font-size: 12px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--croft-color-on-surface-variant);
  }
</style>
