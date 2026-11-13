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
  import { Jmap, explainMail, type Mailbox } from '#lib/jmap/client.ts';
  import { MailState } from '#lib/mail/store.svelte.ts';
  import { ComposeState } from '#lib/mail/compose.svelte.ts';
  import ComposeWindow from '#lib/ui/ComposeWindow.svelte';
  import { Shortcuts, SHORTCUTS, type Command } from '#lib/mail/keys.ts';
  import type { MailAction } from '#lib/mail/actions.ts';
  import Sidebar from '#lib/ui/Sidebar.svelte';
  import MessageRow from '#lib/ui/MessageRow.svelte';
  import ReadingPane from '#lib/ui/ReadingPane.svelte';
  import Snackbar from '#lib/ui/Snackbar.svelte';
  import Dialog from '#lib/ui/Dialog.svelte';
  import IconButton from '#lib/ui/IconButton.svelte';
  import Icon from '#lib/ui/Icon.svelte';
  import { keepPush } from '#lib/pwa/push.ts';
  import { NEW_MAIL, PUSH_CHANGED } from '#lib/pwa/route.ts';

  let me: Me | null = $state(null);
  const jmap = new Jmap(api);
  const mail = new MailState(jmap);
  const compose = new ComposeState(jmap, () => mail.account, (m, u) => mail.notify(m, u));
  let error = $state('');
  /** Opened without a connection to the Companion: not signed out, just offline. */
  let offline = $state(false);
  let noAccounts = $state(false);
  let help = $state(false);
  let picking = $state(false);
  /** An action that deletes for good, waiting for the person to confirm it. */
  let confirming: { action: MailAction; ids: string[] } | null = $state(null);
  const keys = new Shortcuts();

  onMount(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let poll: ReturnType<typeof setInterval> | undefined;
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
    // A tab that slept checks its session and its mail again when it is shown.
    const woke = async () => {
      if (document.visibilityState !== 'visible') return;
      if (!(await api.current())) return void goto('/sign-in', { replaceState: true });
      schedule();
      void mail.refresh().catch(() => {});
    };

    // The service worker's word that mail came (a push ping) or that push needs giving again.
    const fromWorker = (e: MessageEvent) => {
      const type = (e.data as { type?: unknown } | null)?.type;
      if (type === NEW_MAIL && me) void mail.refresh().catch(() => {});
      if (type === PUSH_CHANGED && me) void keepPush(api).catch(() => {});
    };
    navigator.serviceWorker?.addEventListener('message', fromWorker);

    (async () => {
      if (!(await api.current())) {
        if (api.unreachable) {
          offline = true;
          window.addEventListener('online', () => location.reload(), { once: true });
          return;
        }
        return goto('/sign-in', { replaceState: true });
      }
      try {
        me = await api.call<Me>('GET', '/api/me');
      } catch {
        return goto('/sign-in', { replaceState: true });
      }
      if (me.recovery) return goto('/passkey', { replaceState: true });
      schedule();
      // Push on in this browser: the Companion holds this session's current subscription.
      void keepPush(api).catch(() => {});
      try {
        await mail.start();
        noAccounts = mail.accounts.length === 0;
      } catch (e) {
        error = explainMail(e);
      }
      // Fresh mail: every minute while open, and at once on a push ping (T32b).
      poll = setInterval(() => void mail.refresh().catch(() => {}), 60_000);
    })();
    document.addEventListener('visibilitychange', woke);
    return () => {
      stopped = true;
      clearTimeout(timer);
      clearInterval(poll);
      document.removeEventListener('visibilitychange', woke);
      navigator.serviceWorker?.removeEventListener('message', fromWorker);
    };
  });

  async function signOut() {
    await api.signOut().catch(() => {});
    await goto('/sign-in', { replaceState: true });
  }

  function act(a: MailAction, ids: string[] = mail.targets()) {
    if (mail.destroys(a, ids)) {
      confirming = { action: a, ids };
      return;
    }
    void mail.act(a, ids).catch((e) => (error = explainMail(e)));
  }

  function confirmed() {
    const c = confirming;
    confirming = null;
    if (c) void mail.act(c.action, c.ids).catch((e) => (error = explainMail(e)));
  }

  /** The reading pane's toolbar acts on the email it shows, never on the rows ticked in the list. */
  function actOnOpen(a: MailAction) {
    if (mail.open) act(a, [mail.open.id]);
  }

  function moveTo(target: Mailbox) {
    picking = false;
    const ids = mail.open ? [mail.open.id] : mail.targets();
    void mail.act('move', ids, target);
  }

  function run(c: Command) {
    switch (c) {
      case 'next':
        return mail.move(1);
      case 'previous':
        return mail.move(-1);
      case 'open':
        return mail.focused && void mail.openEmail(mail.focused.id);
      case 'back':
        return mail.close();
      case 'select':
        return mail.focused && mail.toggleSelect(mail.focused.id);
      case 'archive':
        return act('archive');
      case 'delete':
        return act('delete');
      case 'mark-read':
        return act('mark-read');
      case 'mark-unread':
        return act('mark-unread');
      case 'star':
        return act('toggle-star');
      case 'undo':
        return void mail.undo();
      case 'go-inbox':
        return void mail.openRole('inbox');
      case 'go-drafts':
        return void mail.openRole('drafts');
      case 'go-sent':
        return void mail.openRole('sent');
      case 'help':
        help = true;
        return;
      case 'compose':
        compose.open();
        return;
      case 'reply':
      case 'reply-all':
      case 'forward': {
        const id = mail.open?.id ?? mail.focused?.id;
        if (id) void compose.reply(c === 'reply' ? 'reply' : c === 'reply-all' ? 'replyAll' : 'forward', id);
        return;
      }
      default:
        // Search comes with T33.
        return;
    }
  }

  function keydown(e: KeyboardEvent) {
    if (!me || help || picking || confirming) return;
    const t = e.target as HTMLElement | null;
    const typing = !!t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName)) && (t as HTMLInputElement).type !== 'checkbox';
    // Enter and Space on a focused button or link are that control's own.
    if ((e.key === 'Enter' || e.key === ' ') && t && ['BUTTON', 'A'].includes(t.tagName)) return;
    const c = keys.press({
      key: e.key,
      shift: e.shiftKey,
      modified: e.ctrlKey || e.altKey || e.metaKey,
      typing,
      timeMs: e.timeStamp
    });
    if (c) {
      e.preventDefault();
      run(c);
    }
  }

  // Keep the keyboard's row in view.
  $effect(() => {
    const id = mail.focused?.id;
    if (id) document.querySelector(`[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest' });
  });

  function scrolled(e: Event) {
    const el = e.currentTarget as HTMLElement;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 400) void mail.more();
  }
</script>

<svelte:window onkeydown={keydown} />

{#if me}
  <div class="w01" data-screen="W01">
    <header class="bar">
      <span class="brand"><Icon name="inbox" /> Croft</span>
      <div class="search" title="Search comes with the Companion's search (T33)">
        <Icon name="search" />
        <span>Search mail</span>
      </div>
      <span class="who" data-testid="user">{me.name}</span>
      <a class="settings" href="/settings" aria-label="Settings" title="Settings"><Icon name="settings" /></a>
      <button class="button text" type="button" onclick={signOut}>Sign out</button>
    </header>

    <Sidebar
      accounts={mail.accounts}
      account={mail.account}
      mailboxes={mail.mailboxes}
      current={mail.mailbox}
      onaccount={(id) => void mail.openAccount(id)}
      onmailbox={(id) => void mail.openMailbox(id)}
      oncompose={() => compose.open()}
    />

    <main class="content">
    <h1 class="sr-only">{mail.current?.name ?? 'Mail'}</h1>
    <section class="list" aria-label="Messages" class:hidden-narrow={mail.open !== null}>
      <div class="list-bar">
        <IconButton icon="refresh" label="Refresh" onclick={() => void mail.refresh()} />
        {#if mail.selected.size > 0}
          <span class="count">{mail.selected.size} selected</span>
          <IconButton icon="archive" label="Archive" shortcut="e" onclick={() => act('archive')} />
          <IconButton icon="delete" label="Delete" shortcut="#" onclick={() => act('delete')} />
          <IconButton icon="read" label="Mark read" shortcut="Shift+i" onclick={() => act('mark-read')} />
        {/if}
        <span class="range">
          {#if mail.total > 0}{Math.min(mail.emails.length, mail.total)} of {mail.total}{/if}
        </span>
      </div>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if noAccounts}
        <p class="muted">No mail accounts yet. Add one in the Croft app on your phone, or ask your admin.</p>
      {/if}
      <ul class="rows" onscroll={scrolled} aria-busy={mail.loading}>
        {#each mail.emails as email, i (email.id)}
          <MessageRow
            {email}
            focused={i === mail.cursor}
            selected={mail.selected.has(email.id)}
            opened={mail.open?.id === email.id}
            can={(a) => mail.can(a, [email.id])}
            onopen={() => void mail.openEmail(email.id)}
            onselect={() => mail.toggleSelect(email.id)}
            onact={(a) => act(a, [email.id])}
          />
        {:else}
          {#if !mail.loading && !noAccounts}<li class="empty muted">Nothing here.</li>{/if}
        {/each}
      </ul>
    </section>

    <ReadingPane
      email={mail.open}
      conversation={mail.conversation}
      onreply={(kind, id) => void compose.reply(kind, id)}
      loading={mail.opening}
      can={(a) => mail.open !== null && mail.can(a, [mail.open.id])}
      onact={actOnOpen}
      onmove={() => (picking = true)}
      onclose={() => mail.close()}
    />
    </main>

    <footer class="hints" aria-label="Keyboard shortcuts">
      <span><kbd>j</kbd>/<kbd>k</kbd> move</span>
      <span><kbd>e</kbd> archive</span>
      <span><kbd>#</kbd> delete</span>
      <span><kbd>z</kbd> undo</span>
      <button type="button" class="link" onclick={() => (help = true)}><kbd>?</kbd> all shortcuts</button>
    </footer>
  </div>

  {#if compose.windows.length > 0}
    <div class="dock" aria-label="Drafts">
      {#each compose.windows as win (win.key)}
        <ComposeWindow {win} {compose} />
      {/each}
    </div>
  {/if}

  {#if mail.snack}
    <Snackbar message={mail.snack.message} canUndo={mail.snack.undo !== null} onundo={() => void mail.undo()} />
  {/if}

  {#if help}
    <Dialog title="Keyboard shortcuts" onclose={() => (help = false)}>
      <table class="shortcuts">
        <tbody>
          {#each SHORTCUTS as [k, what] (k)}
            <tr><td><kbd>{k}</kbd></td><td>{what}</td></tr>
          {/each}
        </tbody>
      </table>
    </Dialog>
  {/if}

  {#if confirming}
    <Dialog title="Delete for good?" onclose={() => (confirming = null)}>
      <p>{confirming.ids.length === 1 ? 'This email' : `These ${confirming.ids.length} emails`} will be deleted for good. This can't be undone.</p>
      <button type="button" class="button danger" onclick={confirmed}>Delete for good</button>
    </Dialog>
  {/if}

  {#if picking}
    <Dialog title="Move to" onclose={() => (picking = false)}>
      <ul class="picker">
        {#each mail.mailboxes.filter((m) => m.id !== mail.mailbox && m.role !== 'outbox' && m.role !== 'drafts') as m (m.id)}
          <li><button type="button" class="button tonal" onclick={() => moveTo(m)}>{m.name}</button></li>
        {/each}
      </ul>
    </Dialog>
  {/if}
{:else if offline}
  <main class="offline-start">
    <h1>You're offline</h1>
    <p>Croft can't reach your Companion. It opens your mail as soon as the connection is back.</p>
  </main>
{:else}
  <p class="loading" aria-busy="true">Loading…</p>
{/if}

<style>
  .w01 {
    height: 100vh;
    display: grid;
    grid-template-columns: 1fr;
    grid-template-rows: auto 1fr auto;
    grid-template-areas: 'bar' 'list' 'hints';
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
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    color: var(--croft-color-primary);
  }

  .search {
    flex: 1;
    max-width: 640px;
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 44px;
    padding: 0 16px;
    border-radius: calc(22px * var(--round));
    background: var(--croft-color-surface-container-high);
    color: var(--croft-color-on-surface-variant);
  }

  .who {
    margin-left: auto;
    color: var(--croft-color-on-surface-variant);
  }

  .w01 :global(.sidebar) {
    display: none;
  }

  .content {
    display: contents;
  }

  .offline-start {
    max-width: 480px;
    margin: 15vh auto;
    padding: 0 16px;
    text-align: center;
  }

  .settings {
    display: grid;
    place-items: center;
    width: var(--croft-touch-min-target-dp);
    height: var(--croft-touch-min-target-dp);
    border-radius: 50%;
    color: inherit;
  }

  .settings:hover {
    background: var(--croft-color-surface-container-high);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .list {
    grid-area: list;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }

  .list-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
  }

  .range {
    margin-left: auto;
    font-size: 13px;
    color: var(--croft-color-on-surface-variant);
  }

  .count {
    font-size: 14px;
  }

  .rows {
    flex: 1;
    margin: 0;
    padding: 0;
    overflow-y: auto;
  }

  .empty {
    padding: 24px;
    list-style: none;
  }

  .w01 :global(.reading) {
    display: none;
  }

  .hints {
    grid-area: hints;
    display: flex;
    gap: 16px;
    padding: 6px 16px;
    font-size: 13px;
    color: var(--croft-color-on-surface-variant);
    background: var(--croft-color-surface-container-low);
  }

  kbd {
    padding: 0 6px;
    border: 1px solid var(--croft-color-outline);
    border-radius: calc(4px * var(--round));
    font-family: inherit;
  }

  .link {
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
  }

  .muted {
    color: var(--croft-color-on-surface-variant);
    padding: 0 16px;
  }

  .error {
    padding: 0 16px;
  }

  .loading {
    padding: 32px;
    text-align: center;
  }

  .dock {
    position: fixed;
    right: 24px;
    bottom: 0;
    display: flex;
    align-items: flex-end;
    gap: 12px;
    max-width: calc(100vw - 48px);
    z-index: 10;
  }

  .shortcuts td {
    padding: 4px 12px 4px 0;
  }

  .picker {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .picker .button {
    width: 100%;
  }

  /* Narrow: the list, or the open email in its place. */
  @media (max-width: 839px) {
    .w01:has(:global(.reading .subject)) :global(.reading) {
      display: flex;
      grid-area: list;
    }

    .hidden-narrow {
      display: none;
    }
  }

  /* Expanded width (840 px and up): sidebar, list and reading pane (W01). */
  @media (min-width: 840px) {
    .w01 {
      grid-template-columns: 256px minmax(320px, 2fr) 3fr;
      grid-template-areas: 'bar bar bar' 'sidebar list reading' 'hints hints hints';
    }

    .w01 :global(.sidebar) {
      display: grid;
      grid-area: sidebar;
    }

    .w01 :global(.reading) {
      display: flex;
      grid-area: reading;
      border-left: 1px solid var(--croft-color-outline-variant);
    }
  }
</style>
