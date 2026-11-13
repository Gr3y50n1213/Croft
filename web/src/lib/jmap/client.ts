// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A small JMAP client (RFC 8620, RFC 8621) for the Companion's `/jmap` API (T27): the session,
 * batched method calls with result references, and typed helpers for what W01 needs. The Companion
 * runs each change through the core's operation queue, so a change shows at once and Undo cancels
 * it while it waits.
 */

import { ApiError, type Api } from '../api.ts';

export const MAIL = 'urn:ietf:params:jmap:mail';
/** The Companion's compose extension (T31b): drafts, replies and the Outbox, by the core's rules. */
export const COMPOSE = 'urn:croft:params:jmap:compose';
const SEARCH = 'urn:croft:params:jmap:search';
const USING = ['urn:ietf:params:jmap:core', MAIL, COMPOSE, SEARCH];

/** A ranged match: `[start, end)` in UTF-16 code units (as JavaScript strings count). */
export type Range = [number, number];

/** What matched in one search result. */
export interface SearchHit {
  /** The subject the ranges count in (as the core stored it). */
  subject?: string;
  subjectRanges: Range[];
  snippet: string | null;
  snippetRanges: Range[];
}

/** A search's answer (`CroftSearch/query`) with the emails as list rows, best first. */
export interface SearchResults {
  emails: EmailSummary[];
  hits: Record<string, SearchHit>;
  /** Some stored emails have no downloaded body, so their text was not searched. */
  bodiesPartial: boolean;
  serverSearched: boolean;
  unsupported: boolean;
  offline: boolean;
  signInNeeded: boolean;
  failed: boolean;
}

/** One mail account (a JMAP account). */
export interface Account {
  id: string;
  name: string;
}

/** A message in the Outbox (`CroftOutbox/get`). */
export interface OutboxItem {
  id: string;
  state: 'queued' | 'sending' | 'failed';
  sendAt: string;
  attempts: number;
  reason: string | null;
  mayHaveBeenSent: boolean;
  draftId: string | null;
  subject: string;
  recipientCount: number;
}

export type Role = 'inbox' | 'archive' | 'drafts' | 'sent' | 'junk' | 'trash' | 'outbox' | null;

export interface Mailbox {
  id: string;
  name: string;
  role: Role;
  totalEmails: number;
  unreadEmails: number;
}

export interface Address {
  name: string | null;
  email: string;
}

/** An email as the list shows it. */
export interface EmailSummary {
  id: string;
  threadId: string;
  mailboxIds: Record<string, boolean>;
  keywords: Record<string, boolean>;
  from: Address[] | null;
  to: Address[] | null;
  cc: Address[] | null;
  subject: string | null;
  receivedAt: string;
  preview: string;
  hasAttachment: boolean;
}

/** An email with its body (the Companion's sanitised HTML, remote content blocked, or text). */
export interface EmailBody extends EmailSummary {
  html: string | null;
  text: string | null;
}

export const SUMMARY_PROPERTIES = [
  'id',
  'threadId',
  'mailboxIds',
  'keywords',
  'from',
  'to',
  'cc',
  'subject',
  'receivedAt',
  'preview',
  'hasAttachment'
] as const;

/** A method call: name, arguments, call id. */
export type Invocation = [string, Record<string, unknown>, string];

/** A draft (the Companion's compose extension). */
export interface DraftFields {
  identityId: string | null;
  to: Address[];
  cc: Address[];
  bcc: Address[];
  subject: string;
  body: string;
  inReplyTo: string | null;
  references: string[];
}

export interface Draft extends DraftFields {
  id: string;
  outboxId: string | null;
}

export type ReplyKind = 'reply' | 'replyAll' | 'forward';

/** A JMAP method error (`["error", {type}, id]`). */
export class MethodError extends Error {
  readonly type: string;
  /** The Companion's `description`, when it names what was wrong (`sendAt`). */
  readonly description: string | undefined;
  constructor(type: string, description?: string) {
    super(type);
    this.type = type;
    this.description = description;
  }
}

/** The order the sidebar lists folders by role (others follow, by name). */
const ROLE_ORDER: Role[] = ['inbox', 'drafts', 'sent', 'outbox', 'archive', 'junk', 'trash'];

export function sortMailboxes(list: Mailbox[]): Mailbox[] {
  const rank = (m: Mailbox) => {
    const i = ROLE_ORDER.indexOf(m.role);
    return i < 0 ? ROLE_ORDER.length : i;
  };
  return [...list].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

export class Jmap {
  #api: Api;

  constructor(api: Api) {
    this.#api = api;
  }

  /** The user's mail accounts, the primary one first. */
  async accounts(): Promise<Account[]> {
    const s = await this.#api.call<{ accounts: Record<string, { name: string }>; primaryAccounts: Record<string, string> }>(
      'GET',
      '/jmap/session'
    );
    const primary = s.primaryAccounts[MAIL];
    const list = Object.entries(s.accounts).map(([id, a]) => ({ id, name: a.name }));
    return list.sort((a, b) => (a.id === primary ? -1 : b.id === primary ? 1 : 0));
  }

  /** Runs method calls in one request; the answers in order. A method error is thrown. */
  async request(calls: Invocation[]): Promise<Invocation[]> {
    const r = await this.#api.call<{ methodResponses: Invocation[] }>('POST', '/jmap/api', {
      using: USING,
      methodCalls: calls
    });
    for (const [name, args] of r.methodResponses) {
      if (name === 'error') {
        const d = args['description'];
        throw new MethodError(String(args['type'] ?? 'serverFail'), typeof d === 'string' ? d : undefined);
      }
    }
    return r.methodResponses;
  }

  async mailboxes(accountId: string): Promise<Mailbox[]> {
    const [[, res]] = (await this.request([['Mailbox/get', { accountId, ids: null }, 'm']])) as [Invocation];
    return sortMailboxes(res['list'] as Mailbox[]);
  }

  /** A page of a mailbox, newest first: the emails and the mailbox's total. */
  /**
   * Searches the account's stored mail (and, with `server`, its mail server) in the core's query
   * language. The rows come back in the search's order.
   */
  async search(accountId: string, query: string, server = false): Promise<SearchResults> {
    const r = await this.request([
      ['CroftSearch/query', { accountId, query, server }, 'q'],
      [
        'Email/get',
        {
          accountId,
          '#ids': { resultOf: 'q', name: 'CroftSearch/query', path: '/ids' },
          properties: SUMMARY_PROPERTIES
        },
        'g'
      ]
    ]);
    const q = r[0]![1];
    const byId = new Map((r[1]![1]['list'] as EmailSummary[]).map((e) => [e.id, e]));
    const emails = (q['ids'] as string[]).map((id) => byId.get(id)).filter((e): e is EmailSummary => !!e);
    return {
      emails,
      hits: q['hits'] as Record<string, SearchHit>,
      bodiesPartial: q['bodiesPartial'] === true,
      serverSearched: q['serverSearched'] === true,
      unsupported: q['unsupported'] === true,
      offline: q['offline'] === true,
      signInNeeded: q['signInNeeded'] === true,
      failed: q['failed'] === true
    };
  }

  async page(
    accountId: string,
    mailboxId: string,
    position: number,
    limit: number
  ): Promise<{ emails: EmailSummary[]; total: number }> {
    const r = await this.request([
      [
        'Email/query',
        {
          accountId,
          filter: { inMailbox: mailboxId },
          sort: [{ property: 'receivedAt', isAscending: false }],
          position,
          limit,
          calculateTotal: true
        },
        'q'
      ],
      [
        'Email/get',
        {
          accountId,
          '#ids': { resultOf: 'q', name: 'Email/query', path: '/ids' },
          properties: SUMMARY_PROPERTIES
        },
        'g'
      ]
    ]);
    const query = r[0]![1];
    const got = r[1]![1];
    return { emails: got['list'] as EmailSummary[], total: Number(query['total'] ?? 0) };
  }

  /** One email with its body. */
  async body(accountId: string, id: string): Promise<EmailBody | null> {
    const [[, res]] = (await this.request([
      [
        'Email/get',
        {
          accountId,
          ids: [id],
          properties: [...SUMMARY_PROPERTIES, 'htmlBody', 'textBody', 'bodyValues'],
          fetchHTMLBodyValues: true,
          fetchTextBodyValues: true
        },
        'b'
      ]
    ])) as [Invocation];
    const e = (res['list'] as Record<string, unknown>[])[0];
    if (!e) return null;
    const values = (e['bodyValues'] ?? {}) as Record<string, { value: string }>;
    const part = (key: 'htmlBody' | 'textBody', type: string) => {
      const parts = (e[key] ?? []) as { partId: string; type: string }[];
      const p = parts.find((x) => x.type === type);
      return p ? (values[p.partId]?.value ?? null) : null;
    };
    return { ...(e as unknown as EmailSummary), html: part('htmlBody', 'text/html'), text: part('textBody', 'text/plain') };
  }

  /** Applies patches (`{id: {keywords/$seen: true}}`, `{id: {mailboxIds: {...}}}`); the ids it refused. */
  async update(accountId: string, update: Record<string, Record<string, unknown>>): Promise<string[]> {
    const [[, res]] = (await this.request([['Email/set', { accountId, update }, 's']])) as [Invocation];
    return Object.keys((res['notUpdated'] ?? {}) as object);
  }

  /** Destroys emails (to Trash, or for good when already in Trash); the ids it refused. */
  async destroy(accountId: string, ids: string[]): Promise<string[]> {
    const [[, res]] = (await this.request([['Email/set', { accountId, destroy: ids }, 'd']])) as [Invocation];
    return Object.keys((res['notDestroyed'] ?? {}) as object);
  }

  /** A conversation's emails with their bodies, oldest first. */
  async thread(accountId: string, threadId: string): Promise<EmailBody[]> {
    const r = await this.request([
      ['Thread/get', { accountId, ids: [threadId] }, 't'],
      [
        'Email/get',
        {
          accountId,
          '#ids': { resultOf: 't', name: 'Thread/get', path: '/list/*/emailIds' },
          properties: [...SUMMARY_PROPERTIES, 'htmlBody', 'textBody', 'bodyValues'],
          fetchHTMLBodyValues: true,
          fetchTextBodyValues: true
        },
        'e'
      ]
    ]);
    const list = (r[1]![1]['list'] ?? []) as Record<string, unknown>[];
    return list.map(withBody).sort((a, b) => a.receivedAt.localeCompare(b.receivedAt));
  }

  /** The fields of a new reply, reply all or forward of `emailId` (by the core's rules; not saved). */
  async replyFields(accountId: string, emailId: string, kind: ReplyKind): Promise<DraftFields> {
    const [[, res]] = (await this.request([['CroftDraft/reply', { accountId, emailId, kind }, 'r']])) as [Invocation];
    return res['draft'] as DraftFields;
  }

  /** Saves a draft (a new one without `id`); its id. */
  async saveDraft(accountId: string, fields: DraftFields, id?: string): Promise<string> {
    const args = id ? { accountId, update: { [id]: fields } } : { accountId, create: { k: fields } };
    const [[, res]] = (await this.request([['CroftDraft/set', args, 's']])) as [Invocation];
    const failed = (id ? res['notUpdated'] : res['notCreated']) as Record<string, { type: string }> | undefined;
    const problem = failed && Object.values(failed)[0];
    if (problem) throw new MethodError(problem.type);
    return id ?? (res['created'] as Record<string, { id: string }>)['k']!.id;
  }

  async discardDraft(accountId: string, id: string): Promise<void> {
    await this.request([['CroftDraft/set', { accountId, destroy: [id] }, 'd']]);
  }

  /**
   * Sends a saved draft: it waits in the Outbox for the undo window, or until `sendAt` (a
   * `UTCDate`) when it is scheduled. Its Outbox id and when it goes.
   */
  async send(accountId: string, draftId: string, sendAt?: string): Promise<{ outboxId: string; sendAt: string }> {
    const args: Record<string, unknown> = { accountId, draftId };
    if (sendAt) args['sendAt'] = sendAt;
    const [[, res]] = (await this.request([['CroftOutbox/send', args, 'o']])) as [Invocation];
    return { outboxId: String(res['outboxId']), sendAt: String(res['sendAt']) };
  }

  /** What waits in the account's Outbox: scheduled messages, ones in their undo window, failures. */
  async outbox(accountId: string): Promise<OutboxItem[]> {
    const [[, res]] = (await this.request([['CroftOutbox/get', { accountId }, 'g']])) as [Invocation];
    return res['list'] as OutboxItem[];
  }

  /** Undoes a send within its window; the draft's id. */
  async undoSend(accountId: string, outboxId: string): Promise<string> {
    const [[, res]] = (await this.request([['CroftOutbox/undo', { accountId, outboxId }, 'u']])) as [Invocation];
    return String(res['draftId']);
  }

  async loadDraft(accountId: string, id: string): Promise<Draft | null> {
    const [[, res]] = (await this.request([['CroftDraft/get', { accountId, ids: [id] }, 'g']])) as [Invocation];
    return ((res['list'] as Draft[])[0] ?? null);
  }
}

function withBody(e: Record<string, unknown>): EmailBody {
  const values = (e['bodyValues'] ?? {}) as Record<string, { value: string }>;
  const part = (key: 'htmlBody' | 'textBody', type: string) => {
    const parts = (e[key] ?? []) as { partId: string; type: string }[];
    const p = parts.find((x) => x.type === type);
    return p ? (values[p.partId]?.value ?? null) : null;
  };
  return { ...(e as unknown as EmailSummary), html: part('htmlBody', 'text/html'), text: part('textBody', 'text/plain') };
}

/** Plain words for a JMAP or API failure. */
export function explainMail(e: unknown): string {
  if (e instanceof ApiError && e.code === 'offline') return "Can't reach your Companion. Check your connection.";
  if (e instanceof ApiError && e.status === 401) return 'Your sign-in ended. Sign in again.';
  if (e instanceof MethodError && e.type === 'requestTooLarge') return 'That is too much at once. Try fewer.';
  if (e instanceof ApiError && e.code === 'rate_limited') return 'Too many requests. Wait a moment.';
  if (e instanceof MethodError && e.type === 'rateLimit') return 'Too many messages sent just now. Wait a minute.';
  if (e instanceof MethodError && e.type === 'noRecipient') return 'Add a recipient.';
  if (e instanceof MethodError && e.type === 'invalidAddress') return 'Fix the address.';
  if (e instanceof MethodError && e.type === 'tooLate') return "Too late: it's already being sent.";
  if (e instanceof MethodError && e.type === 'invalidArguments' && e.description === 'sendAt') return 'Choose a time within a year.';
  if (e instanceof MethodError && e.type === 'invalidArguments' && e.description === 'query')
    return 'Check the search: dates are written like 2026-11-14.';
  if (e instanceof MethodError && e.type === 'rateLimit') return 'Too many searches on the server just now. Wait a minute.';
  return 'Something went wrong. Try again.';
}
