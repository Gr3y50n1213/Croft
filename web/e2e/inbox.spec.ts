// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// W01 (T31a) against a real Companion and the core's IMAP test server: 30 messages ("Message 1" to
// "Message 30") from Ann Example in INBOX, and one HTML email ("Your proof is ready"), the newest. The
// password and keys are made up. Each test has a Companion and a mail server of its own.

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { Companion, TestMail, addAccount, virtualAuthenticator, watchForErrors } from './companion.ts';

let companion: Companion;
let mail: TestMail;

// A Companion and a mailbox per test: every request comes from this machine's one address, and the
// sign-in limits (30 requests a minute per address) would otherwise be shared by all the tests; and
// one test's archive or delete must not change another's mail.
test.beforeEach(async () => {
  mail = await TestMail.start(30);
  companion = await Companion.start({ CROFT_COMPANION_MAIL_ALLOWED_NETWORKS: '127.0.0.1/32' });
});

test.afterEach(() => {
  companion.stop();
  mail.stop();
});

let users = 0;

/** A new user, enrolled, signed in, with the test account added and synced: the inbox shown. */
async function signedInWithMail(page: Page): Promise<void> {
  await virtualAuthenticator(page);
  await page.goto(companion.addUser(`user${++users}`));
  await page.getByRole('button', { name: /passkey/i }).click();
  await expect(page.locator('#step-backup')).toBeVisible();
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  await addAccount(page, mail);
  // The core syncs the account: reload until the Inbox shows its mail.
  await expect(async () => {
    await page.reload();
    await expect(page.locator('.rows li.row').first()).toBeVisible({ timeout: 2000 });
  }).toPass({ timeout: 30_000 });
}

const rows = (page: Page) => page.locator('.rows li.row');
/** The plain-text test messages only ("Message N"). */
const plain = (page: Page) => page.locator('.rows li.row', { hasText: /Message \d+/ });

test('the folders and the Inbox show, and an email opens in the reading pane', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  const nav = page.getByRole('navigation', { name: 'Folders' });
  await expect(nav.getByRole('button', { name: /Inbox/ })).toHaveAttribute('aria-current', 'page');
  await expect(rows(page)).toHaveCount(31);
  await expect(plain(page).first()).toContainText('Ann Example');
  await expect(page.getByText(/of 31/)).toBeVisible();
  await plain(page).first().getByRole('button', { name: /Message/ }).click();
  const pane = page.getByRole('region', { name: 'Reading pane' });
  await expect(pane.getByRole('heading', { level: 2 })).toHaveText(/Message \d+/);
  await expect(pane).toContainText('Hello, this is the body.');
  // Opening it marked it read.
  await expect(plain(page).first()).not.toHaveClass(/unread/);
  expect(errors).toEqual([]);
});

test('the keyboard moves, opens, archives, undoes and stars', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('j');
  await page.keyboard.press('j');
  await expect(rows(page).nth(2)).toHaveClass(/focused/);
  const subject = await rows(page).nth(2).locator('.subject').textContent();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: 'Reading pane' }).getByRole('heading', { level: 2 })).toHaveText(subject!);
  // Archive: the row leaves, the snackbar offers Undo; z puts it back.
  await page.keyboard.press('e');
  await expect(page.getByRole('status')).toContainText('Archived');
  await expect(rows(page)).toHaveCount(30);
  await page.keyboard.press('z');
  await expect(rows(page)).toHaveCount(31);
  await expect(page.locator('.rows .subject', { hasText: subject! })).toHaveCount(1);
  // Star the focused row.
  await page.keyboard.press('k');
  await page.keyboard.press('s');
  await expect(page.locator('.row.focused .star')).toBeVisible();
  // ? shows the shortcuts.
  await page.keyboard.press('Shift+?');
  await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('hover actions delete with Undo, and an archive reaches the Archive folder', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  const first = rows(page).first();
  const subject = (await first.locator('.subject').textContent())!;
  await first.hover();
  await first.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByRole('status')).toContainText('Moved to Trash');
  await expect(page.locator('.rows .subject', { hasText: subject })).toHaveCount(0);
  await page.getByRole('status').getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('.rows .subject', { hasText: subject })).toHaveCount(1);
  // Archive another and find it in Archive once the core has moved it.
  const second = rows(page).nth(1);
  const archived = (await second.locator('.subject').textContent())!;
  await second.hover();
  await second.getByRole('button', { name: 'Archive' }).click();
  await expect(page.locator('.rows .subject', { hasText: archived })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: /Archive/ }).click();
  await expect(async () => {
    await page.getByRole('button', { name: 'Refresh' }).click();
    await expect(page.locator('.rows .subject', { hasText: archived })).toHaveCount(1, { timeout: 2000 });
  }).toPass({ timeout: 20_000 });
  expect(errors).toEqual([]);
});

test('W01 has no accessibility violation (axe-core)', async ({ page }) => {
  await signedInWithMail(page);
  await plain(page).first().getByRole('button', { name: /Message/ }).click();
  await expect(page.getByRole('region', { name: 'Reading pane' }).getByRole('heading', { level: 2 })).toBeVisible();
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + (n.failureSummary ?? "")).join(" | ")}`)).toEqual([]);
});

test('a trackpad two-finger swipe runs the swipe action (right: toggle read, by default)', async ({ page }) => {
  await signedInWithMail(page);
  // The test server's mailbox is shared by every test's user: start from the row's state, whatever it is.
  const row = rows(page).nth(3);
  const wasUnread = /unread/.test((await row.getAttribute('class')) ?? '');
  const box = (await row.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  // A swipe to the right: the content moves right, the wheel reports a negative deltaX.
  for (let i = 0; i < 10; i++) await page.mouse.wheel(-30, 0);
  await expect(row).toHaveClass(wasUnread ? /^(?!.*unread)/ : /unread/);
  // The same swipe again toggles it back.
  for (let i = 0; i < 10; i++) await page.mouse.wheel(-30, 0);
  await expect(row).toHaveClass(wasUnread ? /unread/ : /^(?!.*unread)/);
});

test('an email clicked opens even when the keyboard moves before its body arrives', async ({ page }) => {
  await signedInWithMail(page);
  const subject = (await rows(page).nth(1).locator('.subject').textContent())!;
  await rows(page).nth(1).getByRole('button', { name: /Message/ }).click();
  await page.keyboard.press('j');
  await expect(page.getByRole('region', { name: 'Reading pane' }).getByRole('heading', { level: 2 })).toHaveText(subject);
});

test('an HTML email opens in its sandboxed frame and can do nothing to the app', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  const title = await page.title();
  await page.locator('.rows .subject', { hasText: 'Your proof is ready' }).click();
  const pane = page.getByRole('region', { name: 'Reading pane' });
  await expect(pane.getByRole('heading', { level: 2 })).toHaveText('Your proof is ready');
  const frame = page.frameLocator('iframe.body');
  await expect(frame.locator('body')).toContainText('your proof');
  await expect(frame.getByRole('link', { name: 'Open the proof' })).toBeVisible();
  // Nothing in it ran: the app is untouched.
  await page.waitForTimeout(500);
  expect(await page.title()).toBe(title);
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  // The frame has no script and no same-origin access.
  expect(await page.locator('iframe.body').getAttribute('sandbox')).toBe('allow-popups allow-popups-to-escape-sandbox');
  expect(errors.filter((e) => !e.includes('Refused') && !e.includes('Blocked'))).toEqual([]);
});

test("the reading pane's toolbar acts on the email it shows, not on ticked rows", async ({ page }) => {
  await signedInWithMail(page);
  const ticked = (await rows(page).nth(5).locator('.subject').textContent())!;
  await rows(page).nth(5).getByRole('checkbox').check();
  const shown = (await rows(page).nth(2).locator('.subject').textContent())!;
  await rows(page).nth(2).getByRole('button', { name: /./ }).first().click();
  const pane = page.getByRole('region', { name: 'Reading pane' });
  await expect(pane.getByRole('heading', { level: 2 })).toHaveText(shown);
  await pane.getByRole('button', { name: 'Archive' }).click();
  await expect(page.locator('.rows .subject', { hasText: shown })).toHaveCount(0);
  await expect(page.locator('.rows .subject', { hasText: ticked })).toHaveCount(1);
  await page.getByRole('status').getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('.rows .subject', { hasText: shown })).toHaveCount(1);
});

test("the snackbar's Undo undoes its own action, not a later automatic mark-read", async ({ page }) => {
  await signedInWithMail(page);
  const archived = (await rows(page).nth(1).locator('.subject').textContent())!;
  await rows(page).nth(1).hover();
  await rows(page).nth(1).getByRole('button', { name: 'Archive' }).click();
  await expect(page.getByRole('status')).toContainText('Archived');
  // Open another email (it is marked read without a snackbar), then Undo.
  await rows(page).nth(4).locator('.subject').click();
  await page.getByRole('status').getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('.rows .subject', { hasText: archived })).toHaveCount(1);
  // With no snackbar showing, z does nothing.
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  const count = await rows(page).count();
  await page.keyboard.press('z');
  await expect(rows(page)).toHaveCount(count);
});

test('deleting from Trash asks first, and Cancel keeps the email', async ({ page }) => {
  await signedInWithMail(page);
  const subject = (await rows(page).nth(6).locator('.subject').textContent())!;
  await rows(page).nth(6).hover();
  await rows(page).nth(6).getByRole('button', { name: 'Delete' }).click();
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: /Trash/ }).click();
  await expect(async () => {
    await page.getByRole('button', { name: 'Refresh' }).click();
    await expect(page.locator('.rows .subject', { hasText: subject })).toHaveCount(1, { timeout: 2000 });
  }).toPass({ timeout: 20_000 });
  const row = page.locator('.rows li.row', { hasText: subject });
  await row.hover();
  await row.getByRole('button', { name: 'Delete' }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete for good?' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close' }).click();
  await expect(page.locator('.rows .subject', { hasText: subject })).toHaveCount(1);
  await row.hover();
  await row.getByRole('button', { name: 'Delete' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete for good' }).click();
  await expect(page.locator('.rows .subject', { hasText: subject })).toHaveCount(0);
  await expect(page.getByRole('status')).toContainText('Deleted for good');
});

test('search finds mail by its words, marks the matches, opens a result, and goes back to the folder', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  // `/` goes to the search box.
  await page.keyboard.press('/');
  const box = page.getByRole('searchbox', { name: 'Search mail' });
  await expect(box).toBeFocused();
  await box.fill('proof ready');
  await box.press('Enter');
  await expect(page.getByRole('status').filter({ hasText: /result for “proof ready”/ })).toBeVisible();
  await expect(rows(page)).toHaveCount(1);
  const row = rows(page).first();
  await expect(row.locator('.subject')).toHaveText('Your proof is ready');
  await expect(row.locator('.subject mark')).toHaveText(['proof', 'ready']);
  expect((await new AxeBuilder({ page }).analyze()).violations.map((v) => v.id)).toEqual([]);
  // A result opens as any row does.
  await row.locator('.subject').click();
  await expect(page.getByRole('region', { name: 'Reading pane' }).getByRole('heading', { level: 2 })).toHaveText('Your proof is ready');

  // A query by sender and a phrase: every plain message is from Ann, only one is "Message 7".
  await box.fill('from:ann subject:"Message 7"');
  await box.press('Enter');
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).first().locator('.subject')).toHaveText('Message 7');
  // Most bodies are not downloaded yet, so it offers the mail server too: the same answer, and
  // nothing failed.
  await expect(page.getByText('Searched the headers and the mail downloaded so far.')).toBeVisible();
  await page.getByRole('button', { name: 'Search the server too' }).click();
  await expect(page.getByRole('button', { name: 'Search the server too' })).toHaveCount(0);
  await expect(rows(page)).toHaveCount(1);
  await expect(page.getByText(/didn't answer|can't reach|can't run|sign in again/)).toHaveCount(0);
  // Nothing found says so; a bad date says how to write one.
  await box.fill('nothing-like-this-anywhere');
  await box.press('Enter');
  await expect(page.getByText('No results.')).toBeVisible();
  await box.fill('before:someday');
  await box.press('Enter');
  await expect(page.getByRole('alert')).toHaveText('Check the search: dates are written like 2026-11-14.');

  // Escape: back to the folder.
  await box.press('Escape');
  await expect(rows(page)).toHaveCount(31);
  await expect(box).toHaveValue('');
  expect(errors).toEqual([]);
});
