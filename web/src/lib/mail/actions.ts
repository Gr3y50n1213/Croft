// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * W01's actions on an email (the same set from the toolbar, the hover buttons, the keyboard and a
 * trackpad swipe): what each changes through JMAP, and how Undo puts it back. The core queues every
 * change (moves wait the undo window, read and star 1.5 s), so Undo inside the window cancels the
 * queued operation rather than reversing a finished one (docs/ux/server-contract.md).
 */

import type { EmailSummary, Mailbox, Role } from '../jmap/client.ts';

export type MailAction = 'archive' | 'delete' | 'spam' | 'move' | 'toggle-read' | 'mark-read' | 'mark-unread' | 'toggle-star';

/** One change as JMAP sends it. */
export type Change =
  | { kind: 'update'; patch: Record<string, unknown> }
  | { kind: 'destroy' };

export interface Planned {
  /** The change to make. */
  change: Change;
  /** The change that undoes it (null: nothing to undo, or not undoable). */
  undo: Change | null;
  /** The row leaves the list shown (a move out of it). */
  removesRow: boolean;
  /** The words of the snackbar (null: none, as for read and star). */
  message: string | null;
}

const roleOf = (mailboxes: Mailbox[], role: Role) => mailboxes.find((m) => m.role === role) ?? null;

/** Moves to `target`: the email's mailboxes become just that one; Undo puts the old ones back. */
function move(email: EmailSummary, target: Mailbox, words: string): Planned {
  return {
    change: { kind: 'update', patch: { mailboxIds: { [target.id]: true } } },
    undo: { kind: 'update', patch: { mailboxIds: { ...email.mailboxIds } } },
    removesRow: true,
    message: words
  };
}

/**
 * The plan for `action` on `email` shown in `current` (a mailbox id), or null when it cannot apply
 * there (Archive in Archive, a folder the account does not have).
 */
export function plan(
  action: MailAction,
  email: EmailSummary,
  current: string,
  mailboxes: Mailbox[],
  target?: Mailbox
): Planned | null {
  void current;
  // Where the email is, not which folder is shown: an email is in Trash only by its own mailboxes.
  const isIn = (role: Role) => {
    const m = roleOf(mailboxes, role);
    return m !== null && email.mailboxIds[m.id] === true;
  };
  const seen = email.keywords['$seen'] === true;
  const flagged = email.keywords['$flagged'] === true;
  const keyword = (k: string, on: boolean): Planned => ({
    change: { kind: 'update', patch: { [`keywords/${k}`]: on ? true : null } },
    undo: { kind: 'update', patch: { [`keywords/${k}`]: on ? null : true } },
    removesRow: false,
    message: null
  });
  switch (action) {
    case 'archive': {
      const archive = roleOf(mailboxes, 'archive');
      if (!archive || isIn('archive')) return null;
      return move(email, archive, 'Archived');
    }
    case 'spam': {
      const junk = roleOf(mailboxes, 'junk');
      if (!junk || isIn('junk')) return null;
      return move(email, junk, 'Moved to Spam');
    }
    case 'move':
      if (!target || email.mailboxIds[target.id] === true) return null;
      return move(email, target, `Moved to ${target.name}`);
    case 'delete': {
      const trash = roleOf(mailboxes, 'trash');
      // In Trash, delete is for good: no Undo, so the page asks first (`destroys`).
      if (isIn('trash') || !trash) {
        return { change: { kind: 'destroy' }, undo: null, removesRow: true, message: 'Deleted for good' };
      }
      return move(email, trash, 'Moved to Trash');
    }
    case 'toggle-read':
      return keyword('$seen', !seen);
    case 'mark-read':
      return seen ? null : keyword('$seen', true);
    case 'mark-unread':
      return seen ? keyword('$seen', false) : null;
    case 'toggle-star':
      return keyword('$flagged', !flagged);
  }
}

/** The email as it looks once `change` applied (for the list, before the server confirms). */
export function applied(email: EmailSummary, change: Change): EmailSummary {
  if (change.kind === 'destroy') return email;
  const next: EmailSummary = { ...email, keywords: { ...email.keywords } };
  for (const [path, value] of Object.entries(change.patch)) {
    if (path === 'mailboxIds') next.mailboxIds = { ...(value as Record<string, boolean>) };
    else if (path.startsWith('keywords/')) {
      const k = path.slice('keywords/'.length);
      if (value === true) next.keywords[k] = true;
      else delete next.keywords[k];
    }
  }
  return next;
}

/** Whether `action` would delete any of `emails` for good (the page then asks first). */
export function destroys(action: MailAction, emails: EmailSummary[], mailboxes: Mailbox[]): boolean {
  if (action !== 'delete') return false;
  const trash = roleOf(mailboxes, 'trash');
  return emails.some((e) => !trash || e.mailboxIds[trash.id] === true);
}
