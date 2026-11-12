// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * W01's state: the accounts, the folders, a page of the open folder, the open email, the selection,
 * and the snackbar. Changes show at once (optimistic) and go through JMAP; the core queues them, so
 * Undo within the window cancels them. A refusal puts the row back with a snackbar.
 */

import { Jmap, explainMail, type Account, type EmailBody, type EmailSummary, type Mailbox } from '../jmap/client.ts';
import { applied, plan, type Change, type MailAction } from './actions.ts';

/** Emails per page. */
export const PAGE = 50;
/** How long a snackbar with Undo stays (the default undo window, docs/ux/swipe.md). */
export const UNDO_MS = 5000;

interface Snack {
  message: string;
  undo: (() => Promise<void>) | null;
  id: number;
}

export class MailState {
  readonly #jmap: Jmap;
  accounts: Account[] = $state([]);
  account: string | null = $state(null);
  mailboxes: Mailbox[] = $state([]);
  mailbox: string | null = $state(null);
  emails: EmailSummary[] = $state([]);
  total = $state(0);
  /** The row the keyboard is on. */
  cursor = $state(0);
  selected: Set<string> = $state(new Set());
  open: EmailBody | null = $state(null);
  opening = $state(false);
  loading = $state(false);
  snack: Snack | null = $state(null);
  #snackTimer: ReturnType<typeof setTimeout> | undefined;
  #snackId = 0;
  /** The last undoable action, for `z` and the snackbar. */
  #lastUndo: (() => Promise<void>) | null = null;

  constructor(jmap: Jmap) {
    this.#jmap = jmap;
  }

  get current(): Mailbox | null {
    return this.mailboxes.find((m) => m.id === this.mailbox) ?? null;
  }

  get focused(): EmailSummary | null {
    return this.emails[this.cursor] ?? null;
  }

  /** The first account's Inbox. */
  async start(): Promise<void> {
    this.accounts = await this.#jmap.accounts();
    const first = this.accounts[0];
    if (!first) return;
    await this.openAccount(first.id);
  }

  async openAccount(id: string): Promise<void> {
    this.account = id;
    this.mailboxes = await this.#jmap.mailboxes(id);
    const inbox = this.mailboxes.find((m) => m.role === 'inbox') ?? this.mailboxes[0];
    if (inbox) await this.openMailbox(inbox.id);
  }

  async openRole(role: Mailbox['role']): Promise<void> {
    const m = this.mailboxes.find((x) => x.role === role);
    if (m) await this.openMailbox(m.id);
  }

  async openMailbox(id: string): Promise<void> {
    if (!this.account) return;
    this.mailbox = id;
    this.open = null;
    this.cursor = 0;
    this.selected = new Set();
    this.loading = true;
    try {
      const page = await this.#jmap.page(this.account, id, 0, PAGE);
      if (this.mailbox !== id) return;
      this.emails = page.emails;
      this.total = page.total;
    } finally {
      this.loading = false;
    }
  }

  /** The next page, appended. */
  async more(): Promise<void> {
    if (!this.account || !this.mailbox || this.loading || this.emails.length >= this.total) return;
    const id = this.mailbox;
    this.loading = true;
    try {
      const page = await this.#jmap.page(this.account, id, this.emails.length, PAGE);
      if (this.mailbox !== id) return;
      const have = new Set(this.emails.map((e) => e.id));
      this.emails = [...this.emails, ...page.emails.filter((e) => !have.has(e.id))];
      this.total = page.total;
    } finally {
      this.loading = false;
    }
  }

  /** Reloads the folders and the first pages shown (on focus, on push, every minute). */
  async refresh(): Promise<void> {
    if (!this.account || !this.mailbox || this.loading) return;
    const id = this.mailbox;
    const [boxes, page] = await Promise.all([
      this.#jmap.mailboxes(this.account),
      this.#jmap.page(this.account, id, 0, Math.max(PAGE, this.emails.length))
    ]);
    if (this.mailbox !== id) return;
    this.mailboxes = boxes;
    const focusedId = this.focused?.id;
    this.emails = page.emails;
    this.total = page.total;
    const at = focusedId ? this.emails.findIndex((e) => e.id === focusedId) : -1;
    this.cursor = at >= 0 ? at : Math.min(this.cursor, Math.max(0, this.emails.length - 1));
  }

  /** Opens an email in the reading pane, and marks it read. */
  async openEmail(id: string): Promise<void> {
    if (!this.account) return;
    const at = this.emails.findIndex((e) => e.id === id);
    if (at >= 0) this.cursor = at;
    this.opening = true;
    try {
      const body = await this.#jmap.body(this.account, id);
      if (this.focused?.id !== id && at >= 0) return;
      this.open = body;
    } finally {
      this.opening = false;
    }
    const email = this.emails.find((e) => e.id === id);
    if (email && email.keywords['$seen'] !== true) await this.act('mark-read', [id]);
  }

  close(): void {
    this.open = null;
  }

  move(by: number): void {
    if (this.emails.length === 0) return;
    this.cursor = Math.max(0, Math.min(this.emails.length - 1, this.cursor + by));
    if (by > 0 && this.cursor >= this.emails.length - 5) void this.more();
  }

  toggleSelect(id: string): void {
    const next = new Set(this.selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.selected = next;
  }

  /** The emails an action applies to: the selection, or the open email, or the focused row. */
  targets(): string[] {
    if (this.selected.size > 0) return [...this.selected];
    const id = this.open?.id ?? this.focused?.id;
    return id ? [id] : [];
  }

  /** Whether `action` applies to the current targets (for the toolbar and hover buttons). */
  can(action: MailAction, ids: string[] = this.targets()): boolean {
    if (!this.mailbox) return false;
    return ids.some((id) => {
      const e = this.emails.find((x) => x.id === id) ?? (this.open?.id === id ? this.open : null);
      return e !== null && plan(action, e, this.mailbox!, this.mailboxes) !== null;
    });
  }

  /** Runs `action` on `ids` (default: the targets). */
  async act(action: MailAction, ids: string[] = this.targets(), target?: Mailbox): Promise<void> {
    const account = this.account;
    const mailbox = this.mailbox;
    if (!account || !mailbox) return;
    const plans: { email: EmailSummary; index: number; change: Change; undo: Change | null; removes: boolean; message: string | null }[] = [];
    for (const id of ids) {
      const index = this.emails.findIndex((e) => e.id === id);
      const email = this.emails[index] ?? (this.open?.id === id ? this.open : null);
      if (!email) continue;
      const p = plan(action, email, mailbox, this.mailboxes, target);
      if (p) plans.push({ email, index, change: p.change, undo: p.undo, removes: p.removesRow, message: p.message });
    }
    if (plans.length === 0) return;
    // At once on screen.
    const removedIds = new Set(plans.filter((p) => p.removes).map((p) => p.email.id));
    const changed = new Map(plans.filter((p) => !p.removes).map((p) => [p.email.id, applied(p.email, p.change)]));
    this.emails = this.emails.filter((e) => !removedIds.has(e.id)).map((e) => changed.get(e.id) ?? e);
    if (this.open && removedIds.has(this.open.id)) this.open = null;
    else if (this.open && changed.has(this.open.id)) this.open = { ...this.open, ...changed.get(this.open.id)! };
    this.selected = new Set([...this.selected].filter((id) => !removedIds.has(id)));
    this.cursor = Math.min(this.cursor, Math.max(0, this.emails.length - 1));
    this.total -= removedIds.size;
    // Then through JMAP.
    let refused: string[] = [];
    try {
      const updates = Object.fromEntries(
        plans.filter((p) => p.change.kind === 'update').map((p) => [p.email.id, (p.change as { patch: Record<string, unknown> }).patch])
      );
      const destroys = plans.filter((p) => p.change.kind === 'destroy').map((p) => p.email.id);
      if (Object.keys(updates).length) refused = refused.concat(await this.#jmap.update(account, updates));
      if (destroys.length) refused = refused.concat(await this.#jmap.destroy(account, destroys));
    } catch (e) {
      refused = plans.map((p) => p.email.id);
      this.#show(explainMail(e), null);
    }
    if (refused.length > 0) {
      // Put back what the server refused, where it was.
      this.#restore(plans.filter((p) => refused.includes(p.email.id)), mailbox);
      this.#show(`Couldn't ${verb(action)}. Try again.`, null);
      return;
    }
    const message = plans.length > 1 ? `${plans[0]!.message ?? ''} (${plans.length})` : plans[0]!.message;
    const undoable = plans.filter((p) => p.undo);
    const undo =
      undoable.length > 0
        ? async () => {
            const updates = Object.fromEntries(undoable.map((p) => [p.email.id, (p.undo as { patch: Record<string, unknown> }).patch]));
            this.#restore(undoable, mailbox);
            await this.#jmap.update(account, updates).catch(() => this.#show("Couldn't undo.", null));
          }
        : null;
    this.#lastUndo = undo;
    if (message) this.#show(message, undo);
  }

  /** Puts rows back in the folder shown (if it still is) at their old places, as they were. */
  #restore(plans: { email: EmailSummary; index: number; removes: boolean }[], mailbox: string): void {
    if (this.mailbox !== mailbox) return;
    let list = [...this.emails];
    for (const p of [...plans].sort((a, b) => a.index - b.index)) {
      if (p.removes) {
        if (list.some((e) => e.id === p.email.id)) continue;
        list.splice(Math.max(0, Math.min(p.index, list.length)), 0, p.email);
        this.total += 1;
      } else {
        list = list.map((e) => (e.id === p.email.id ? p.email : e));
      }
    }
    this.emails = list;
  }

  /** `z`: undoes the last action, once. */
  async undo(): Promise<void> {
    const u = this.#lastUndo;
    this.#lastUndo = null;
    this.snack = null;
    if (u) await u();
  }

  #show(message: string, undo: (() => Promise<void>) | null): void {
    clearTimeout(this.#snackTimer);
    const id = ++this.#snackId;
    this.snack = { message, undo, id };
    this.#snackTimer = setTimeout(() => {
      if (this.snack?.id === id) this.snack = null;
    }, UNDO_MS);
  }

  dismiss(): void {
    this.snack = null;
  }
}

function verb(action: MailAction): string {
  switch (action) {
    case 'archive':
      return 'archive';
    case 'delete':
      return 'delete';
    case 'spam':
      return 'move to Spam';
    case 'move':
      return 'move';
    case 'toggle-star':
      return 'star';
    default:
      return 'change it';
  }
}
