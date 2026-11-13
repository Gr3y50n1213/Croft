// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * W02's compose windows: up to three at once, each a draft the core keeps (saved a second after
 * typing stops, and on close). Send puts the draft in the Outbox for the undo window; Undo brings
 * it back into a window. Replies and forwards get their fields from the core (`CroftDraft/reply`).
 */

import { explainMail, type DraftFields, type Jmap, type ReplyKind } from '../jmap/client.ts';

/** Windows open at once (docked along the bottom). */
export const MAX_WINDOWS = 3;
/** A draft is saved this long after typing stops (docs/ux/state-machines.md, "Outgoing message"). */
export const SAVE_AFTER_MS = 1000;

export interface ComposeWindow {
  key: number;
  draftId: string | null;
  fields: DraftFields;
  minimized: boolean;
  status: string;
  sending: boolean;
}

const empty = (): DraftFields => ({
  identityId: null,
  to: [],
  cc: [],
  bcc: [],
  subject: '',
  body: '',
  inReplyTo: null,
  references: []
});

export class ComposeState {
  readonly #jmap: Jmap;
  readonly #account: () => string | null;
  readonly #notify: (message: string, undo: (() => Promise<void>) | null) => void;
  windows: ComposeWindow[] = $state([]);
  #next = 1;
  #timers = new Map<number, ReturnType<typeof setTimeout>>();

  constructor(
    jmap: Jmap,
    account: () => string | null,
    notify: (message: string, undo: (() => Promise<void>) | null) => void
  ) {
    this.#jmap = jmap;
    this.#account = account;
    this.#notify = notify;
  }

  /** Opens a window (new, or with fields). False when three are open already. */
  open(fields: DraftFields = empty(), draftId: string | null = null): boolean {
    if (this.windows.length >= MAX_WINDOWS) {
      this.#notify('Close a draft first: three are open.', null);
      return false;
    }
    for (const w of this.windows) w.minimized = true;
    this.windows.push({ key: this.#next++, draftId, fields, minimized: false, status: '', sending: false });
    return true;
  }

  /** A reply, reply all or forward of `emailId`, by the core's rules. */
  async reply(kind: ReplyKind, emailId: string): Promise<void> {
    const account = this.#account();
    if (!account || this.windows.length >= MAX_WINDOWS) {
      this.open();
      return;
    }
    try {
      this.open(await this.#jmap.replyFields(account, emailId, kind));
    } catch (e) {
      this.#notify(explainMail(e), null);
    }
  }

  #find(key: number): ComposeWindow | undefined {
    return this.windows.find((w) => w.key === key);
  }

  /** A field changed: saved a second after typing stops. */
  edited(key: number): void {
    const w = this.#find(key);
    if (!w) return;
    w.status = '';
    clearTimeout(this.#timers.get(key));
    this.#timers.set(
      key,
      setTimeout(() => void this.save(key), SAVE_AFTER_MS)
    );
  }

  /** Saves the window's draft now. */
  async save(key: number): Promise<boolean> {
    clearTimeout(this.#timers.get(key));
    const w = this.#find(key);
    const account = this.#account();
    if (!w || !account) return false;
    try {
      w.draftId = await this.#jmap.saveDraft(account, $state.snapshot(w.fields), w.draftId ?? undefined);
      w.status = 'Saved';
      return true;
    } catch (e) {
      w.status = explainMail(e);
      return false;
    }
  }

  toggle(key: number): void {
    const w = this.#find(key);
    if (w) w.minimized = !w.minimized;
  }

  /** Closes the window; what was written stays a draft. */
  async close(key: number): Promise<void> {
    const w = this.#find(key);
    if (!w) return;
    const hasContent = w.fields.to.length + w.fields.cc.length + w.fields.bcc.length > 0 || w.fields.subject || w.fields.body;
    if (hasContent) await this.save(key);
    this.#drop(key);
  }

  /** Discards the window and its draft. */
  async discard(key: number): Promise<void> {
    const w = this.#find(key);
    const account = this.#account();
    if (!w) return;
    this.#drop(key);
    if (w.draftId && account) await this.#jmap.discardDraft(account, w.draftId).catch(() => {});
  }

  #drop(key: number): void {
    clearTimeout(this.#timers.get(key));
    this.#timers.delete(key);
    this.windows = this.windows.filter((w) => w.key !== key);
  }

  /** Saves and sends: into the Outbox for the undo window, with Undo in the snackbar. */
  async send(key: number): Promise<void> {
    const w = this.#find(key);
    const account = this.#account();
    if (!w || !account || w.sending) return;
    w.sending = true;
    try {
      if (!(await this.save(key)) || !w.draftId) return;
      const { outboxId } = await this.#jmap.send(account, w.draftId);
      this.#drop(key);
      this.#notify('Sending…', async () => {
        try {
          const draftId = await this.#jmap.undoSend(account, outboxId);
          const draft = await this.#jmap.loadDraft(account, draftId);
          if (draft) this.open(draft, draftId);
        } catch (e) {
          this.#notify(explainMail(e), null);
        }
      });
    } catch (e) {
      w.status = explainMail(e);
    } finally {
      w.sending = false;
    }
  }
}
