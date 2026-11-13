<!--
  This Source Code Form is subject to the terms of the Mozilla Public
  License, v. 2.0. If a copy of the MPL was not distributed with this
  file, You can obtain one at https://mozilla.org/MPL/2.0/.
-->
<script lang="ts" module>
  /**
   * The page requires Trusted Types (its CSP): the email frame's document goes through a policy of
   * its own, named in the CSP and used for nothing else. The frame's sandbox (no script, opaque
   * origin) is what keeps the email's HTML harmless; the policy only lets this one sink take it.
   */
  interface TrustedTypesLike {
    createPolicy(name: string, rules: { createHTML: (s: string) => string }): { createHTML(s: string): unknown };
  }
  const tt = (globalThis as { trustedTypes?: TrustedTypesLike }).trustedTypes;
  const policy = tt?.createPolicy('croft-mail-srcdoc', { createHTML: (s: string) => s });
  const trusted = (doc: string): string => (policy ? policy.createHTML(doc) : doc) as string;
</script>

<script lang="ts">
  import IconButton from './IconButton.svelte';
  import type { EmailBody, ReplyKind } from '../jmap/client.ts';
  import type { MailAction } from '../mail/actions.ts';

  let {
    email,
    conversation,
    loading,
    can,
    onact,
    onmove,
    onclose,
    onreply
  }: {
    email: EmailBody | null;
    conversation: EmailBody[];
    loading: boolean;
    can: (a: MailAction) => boolean;
    onact: (a: MailAction) => void;
    onmove: () => void;
    onclose: () => void;
    onreply: (kind: ReplyKind, emailId: string) => void;
  } = $props();

  /** Earlier messages of the conversation, collapsed unless opened. */
  let expanded: Set<string> = $state(new Set());
  const earlier = $derived(conversation.filter((e) => e.id !== email?.id));
  const docOf = (e: EmailBody) =>
    e.html
      ? trusted(
          `<!doctype html><html><head><meta charset="utf-8"><meta name="referrer" content="no-referrer"><base target="_blank"></head><body>${e.html}</body></html>`
        )
      : null;

  const seen = $derived(email?.keywords['$seen'] === true);
  const flagged = $derived(email?.keywords['$flagged'] === true);
  const who = (list: EmailBody['from']) => (list ?? []).map((a) => (a.name ? `${a.name} <${a.email}>` : a.email)).join(', ');

  /**
   * The message's HTML, already sanitised by the Companion (no script, remote content blocked), in a
   * frame with no permissions at all: no script, no same-origin access, no forms; links open in a
   * new tab without a referrer. The page's CSP also applies inside it.
   */
</script>

{#snippet body(e: EmailBody)}
  {@const d = docOf(e)}
  {#if d}
    <iframe
      class="body"
      title={`Message: ${e.subject ?? ''}`}
      sandbox="allow-popups allow-popups-to-escape-sandbox"
      referrerpolicy="no-referrer"
      srcdoc={d}
    ></iframe>
  {:else}
    <pre class="text">{e.text ?? ''}</pre>
  {/if}
{/snippet}

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
    {#each earlier as e (e.id)}
      <div class="earlier">
        <button
          type="button"
          class="summary"
          aria-expanded={expanded.has(e.id)}
          onclick={() => {
            const next = new Set(expanded);
            if (next.has(e.id)) next.delete(e.id);
            else next.add(e.id);
            expanded = next;
          }}
        >
          <span class="from">{who(e.from)}</span>
          <span class="date">{new Date(e.receivedAt).toLocaleString()}</span>
          {#if !expanded.has(e.id)}<span class="snippet">{e.preview}</span>{/if}
        </button>
        {#if expanded.has(e.id)}{@render body(e)}{/if}
      </div>
    {/each}
    {#if email.to?.length}<p class="meta">To {who(email.to)}</p>{/if}
    {#if email.hasAttachment}<p class="meta">This message has attachments (downloads come later).</p>{/if}
    {@render body(email)}
    <div class="replies">
      <button type="button" class="button tonal" title="Reply (r)" onclick={() => onreply('reply', email.id)}>Reply</button>
      <button type="button" class="button tonal" title="Reply all (a)" onclick={() => onreply('replyAll', email.id)}>Reply all</button>
      <button type="button" class="button tonal" title="Forward (f)" onclick={() => onreply('forward', email.id)}>Forward</button>
    </div>
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
    border-radius: calc(12px * var(--round));
    background: var(--croft-color-surface-container-lowest);
  }

  .earlier {
    display: grid;
    gap: 8px;
    padding: 8px 0;
    border-bottom: 1px solid var(--croft-color-outline-variant);
  }

  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 4px 0;
    border: none;
    background: none;
    text-align: start;
    cursor: pointer;
  }

  .snippet {
    flex-basis: 100%;
    color: var(--croft-color-on-surface-variant);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .replies {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    padding: 8px 0 24px;
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
