<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts">
  import IconButton from './IconButton.svelte';
  import type { EmailBody } from '../jmap/client.ts';
  import type { MailAction } from '../mail/actions.ts';

  let {
    email,
    loading,
    can,
    onact,
    onmove,
    onclose
  }: {
    email: EmailBody | null;
    loading: boolean;
    can: (a: MailAction) => boolean;
    onact: (a: MailAction) => void;
    onmove: () => void;
    onclose: () => void;
  } = $props();

  const seen = $derived(email?.keywords['$seen'] === true);
  const flagged = $derived(email?.keywords['$flagged'] === true);
  const who = (list: EmailBody['from']) => (list ?? []).map((a) => (a.name ? `${a.name} <${a.email}>` : a.email)).join(', ');

  /**
   * The message's HTML, already sanitised by the Companion (no script, remote content blocked), in a
   * frame with no permissions at all: no script, no same-origin access, no forms; links open in a
   * new tab without a referrer. The page's CSP also applies inside it.
   */
  const doc = $derived(
    email?.html
      ? `<!doctype html><html><head><meta charset="utf-8"><meta name="referrer" content="no-referrer"><base target="_blank"></head><body>${email.html}</body></html>`
      : null
  );
</script>

<section class="reading" aria-label="Reading pane">
  {#if email}
    <div class="toolbar" role="toolbar" aria-label="Actions">
      <IconButton icon="back" label="Back to the list" shortcut="u" onclick={onclose} />
      <IconButton icon="archive" label="Archive" shortcut="e" disabled={!can('archive')} onclick={() => onact('archive')} />
      <IconButton icon="delete" label="Delete" shortcut="#" disabled={!can('delete')} onclick={() => onact('delete')} />
      <IconButton
        icon={seen ? 'unread' : 'read'}
        label={seen ? 'Mark unread' : 'Mark read'}
        shortcut={seen ? 'Shift+u' : 'Shift+i'}
        onclick={() => onact(seen ? 'mark-unread' : 'mark-read')}
      />
      <IconButton icon="star" filled={flagged} label={flagged ? 'Unstar' : 'Star'} shortcut="s" onclick={() => onact('toggle-star')} />
      <IconButton icon="folder" label="Move to" onclick={onmove} />
      <IconButton icon="spam" label="Spam" disabled={!can('spam')} onclick={() => onact('spam')} />
    </div>
    <h2 class="subject">{email.subject || '(no subject)'}</h2>
    <p class="meta">
      <span class="from">{who(email.from)}</span>
      <span class="date">{new Date(email.receivedAt).toLocaleString()}</span>
    </p>
    {#if email.to?.length}<p class="meta">To {who(email.to)}</p>{/if}
    {#if email.hasAttachment}<p class="meta">This message has attachments (downloads come later).</p>{/if}
    {#if doc}
      <iframe
        class="body"
        title={`Message: ${email.subject ?? ''}`}
        sandbox="allow-popups allow-popups-to-escape-sandbox"
        referrerpolicy="no-referrer"
        srcdoc={doc}
      ></iframe>
    {:else}
      <pre class="text">{email.text ?? ''}</pre>
    {/if}
  {:else if loading}
    <p class="muted" aria-busy="true">Opening…</p>
  {:else}
    <p class="muted">Pick a conversation to read it.</p>
  {/if}
</section>

<style>
  .reading {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    padding: 8px 24px 16px;
    overflow-y: auto;
  }

  .toolbar {
    display: flex;
    gap: 4px;
  }

  .subject {
    margin: 8px 0 0;
    font-size: 24px;
    font-weight: 500;
  }

  .meta {
    display: flex;
    gap: 12px;
    margin: 0;
    color: var(--croft-color-on-surface-variant);
    font-size: 14px;
  }

  .from {
    flex: 1;
    color: var(--croft-color-on-surface);
  }

  .body {
    flex: 1;
    min-height: 360px;
    width: 100%;
    border: none;
    border-radius: 12px;
    background: var(--croft-color-surface-container-lowest);
  }

  .text {
    margin: 0;
    white-space: pre-wrap;
    font: inherit;
  }

  .muted {
    color: var(--croft-color-on-surface-variant);
  }
</style>
