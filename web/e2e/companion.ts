// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { spawn, execFileSync, type ChildProcess } from 'node:child_process';
import { createHmac } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import type { Page } from '@playwright/test';

const BIN = process.env['CROFT_COMPANION_BIN'] ?? resolve('../companion/target/debug/croft-companion');

/** A Companion of its own for one test file, serving the built app on http://localhost:<port>. */
export class Companion {
  readonly url: string;
  #dir: string;
  #env: NodeJS.ProcessEnv;
  #proc: ChildProcess | null = null;

    private constructor(port: number, extra: NodeJS.ProcessEnv = {}) {
    this.#dir = mkdtempSync(join(tmpdir(), 'croft-e2e-'));
    this.url = `http://localhost:${port}`;
    this.#env = {
      ...process.env,
      CROFT_COMPANION_LISTEN: `127.0.0.1:${port}`,
      CROFT_COMPANION_PUBLIC_URL: this.url,
      CROFT_COMPANION_DATA_DIR: join(this.#dir, 'data'),
      CROFT_COMPANION_MASTER_KEY_FILE: join(this.#dir, 'keys', 'master.key'),
            CROFT_COMPANION_WEB_DIR: resolve('build'),
      ...extra
    };
  }

  /** `extra`: more settings, such as `CROFT_COMPANION_MAIL_ALLOWED_NETWORKS` for a mail server here. */
  static async start(extra: NodeJS.ProcessEnv = {}): Promise<Companion> {
    const c = new Companion(await freePort(), extra);
    c.cli('init');
    c.#proc = spawn(BIN, ['serve'], { env: c.#env, stdio: ['ignore', 'ignore', 'inherit'] });
    for (let i = 0; i < 100; i++) {
      try {
        if ((await fetch(`${c.url}/healthz`)).ok) return c;
      } catch {
        // Not listening yet.
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error('the Companion did not start');
  }

  cli(...args: string[]): string {
    return execFileSync(BIN, args, { env: this.#env, encoding: 'utf8' });
  }

  /** Adds a user; their enrolment link. */
  addUser(name: string): string {
    const out = this.cli('user', 'add', '--name', name);
    const link = out.split('\n').find((l) => l.startsWith('http'));
    if (!link) throw new Error('no enrolment link');
    return link.trim();
  }

  stop(): void {
    this.#proc?.kill('SIGTERM');
    rmSync(this.#dir, { recursive: true, force: true });
  }
}

function freePort(): Promise<number> {
  return new Promise((ok, fail) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => {
      const a = s.address();
      s.close(() => (a && typeof a === 'object' ? ok(a.port) : fail(new Error('no port'))));
    });
  });
}

/**
 * A passkey for the page: Chromium's virtual authenticator (made-up keys, this test only). Returns
 * a function that removes it (the passkeys on it are then lost, as with a lost phone).
 */
export async function virtualAuthenticator(page: Page): Promise<() => Promise<void>> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true
    }
  });
  return async () => {
    await cdp.send('WebAuthn.removeVirtualAuthenticator', { authenticatorId });
  };
}

/** The TOTP code (RFC 6238, SHA-1, 6 digits, 30 s) for a base32 secret. */
export function totp(secret: string, now = Date.now()): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const c of secret.replace(/=+$/, '').toUpperCase()) bits += alphabet.indexOf(c).toString(2).padStart(5, '0');
  const key = Buffer.from(bits.match(/.{8}/g)!.map((b) => parseInt(b, 2)));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(now / 30_000)));
  const h = createHmac('sha1', key).update(counter).digest();
  const o = h[h.length - 1]! & 15;
  return String((h.readUInt32BE(o) & 0x7fffffff) % 1_000_000).padStart(6, '0');
}

/** Fails the test on any CSP violation or script error the page reports. */
export function watchForErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (m) => {
    // A refused request is logged by the browser too; those the app expects (no session yet) are
    // not errors of the page.
    if (m.type() === 'error' && !m.text().startsWith('Failed to load resource')) errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(String(e)));
  return errors;
}

const MAIL_BIN = process.env['CROFT_TEST_MAIL_BIN'] ?? resolve('../companion/target/debug/croft-test-mail');

/** The core's IMAP test server (`croft-test-mail`, test only) with `count` messages. */
export class TestMail {
  readonly host: string;
  readonly port: number;
  readonly username: string;
  readonly password: string;
  readonly fingerprint: string;
  #proc: ChildProcess;

  private constructor(proc: ChildProcess, info: Record<string, string | number>) {
    this.#proc = proc;
    this.host = String(info['host']);
    this.port = Number(info['port']);
    this.username = String(info['username']);
    this.password = String(info['password']);
    this.fingerprint = String(info['fingerprint']);
  }

  static start(count = 30): Promise<TestMail> {
    const proc = spawn(MAIL_BIN, [String(count)], { stdio: ['pipe', 'pipe', 'inherit'] });
    return new Promise((ok, fail) => {
      let out = '';
      proc.stdout!.on('data', (d: Buffer) => {
        out += d.toString();
        const line = out.split('\n')[0];
        if (out.includes('\n') && line) ok(new TestMail(proc, JSON.parse(line)));
      });
      proc.on('exit', () => fail(new Error('croft-test-mail stopped')));
    });
  }

  stop(): void {
    this.#proc.stdin?.end();
    this.#proc.kill('SIGTERM');
  }
}

/** Adds the test server's account to the signed-in user of `page` (through the API, as the phone app does). */
export async function addAccount(page: Page, mail: TestMail): Promise<void> {
  const status = await page.evaluate(async (m) => {
    // The web app keeps its access token to itself: take a fresh one through the cookie.
    const r = await fetch('/api/web/refresh', { method: 'POST', headers: { 'x-croft-web': '1' } });
    const { access_token } = await r.json();
    const add = await fetch('/api/accounts', {
      method: 'POST',
      headers: { authorization: `Bearer ${access_token}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        discovered: { email: 'alice@example.test', imap: { host: m.host, port: m.port, security: 'tls', username: m.username } },
        password: m.password,
        pin: m.fingerprint
      })
    });
    return `${add.status} ${await add.text()}`;
  }, { host: mail.host, port: mail.port, username: mail.username, password: mail.password, fingerprint: mail.fingerprint });
  if (!status.startsWith('200')) throw new Error(`adding the account: ${status}`);
}
