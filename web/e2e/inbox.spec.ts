// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// W01 (T31a) against a real Companion and the core's IMAP test server: 30 messages ("Message 1" to
// "Message 30") from Ann Example in INBOX. The password and keys are made up.

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { Companion, TestMail, addAccount, virtualAuthenticator, watchForErrors } from './companion.ts';

let companion: Companion;
let mail: TestMail;

test.beforeAll(async () => {
  mail = await TestMail.start(30);
  companion = await Companion.start({ CROFT_COMPANION_MAIL_ALLOWED_NETWORKS: '127.0.0.1/32' });
});

test.afterAll(() => {
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

test('the folders and the Inbox show, and an email opens in the reading pane', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  const nav = page.getByRole('navigation', { name: 'Folders' });
  await expect(nav.getByRole('button', { name: /Inbox/ })).toHaveAttribute('aria-current', 'page');
  await expect(rows(page)).toHaveCount(30);
  await expect(rows(page).first()).toContainText('Ann Example');
  await expect(page.getByText(/of 30/)).toBeVisible();
  await rows(page).first().getByRole('button', { name: /Message/ }).click();
  const pane = page.getByRole('region', { name: 'Reading pane' });
  await expect(pane.getByRole('heading', { level: 2 })).toHaveText(/Message \d+/);
  await expect(pane).toContainText('Hello, this is the body.');
  // Opening it marked it read.
  await expect(rows(page).first()).not.toHaveClass(/unread/);
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
  await expect(rows(page)).toHaveCount(29);
  await page.keyboard.press('z');
  await expect(rows(page)).toHaveCount(30);
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
  await rows(page).first().getByRole('button', { name: /Message/ }).click();
  await expect(page.getByRole('region', { name: 'Reading pane' }).getByRole('heading', { level: 2 })).toBeVisible();
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + (n.failureSummary ?? "")).join(" | ")}`)).toEqual([]);
});

test('a trackpad two-finger swipe runs the swipe action (right: mark read, by default)', async ({ page }) => {
  await signedInWithMail(page);
  const row = rows(page).nth(3);
  await expect(row).toHaveClass(/unread/);
  const box = (await row.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  // A swipe to the right: the content moves right, the wheel reports a negative deltaX.
  for (let i = 0; i < 10; i++) await page.mouse.wheel(-30, 0);
  await expect(row).not.toHaveClass(/unread/);
  // The same swipe again toggles it back: unread.
  for (let i = 0; i < 10; i++) await page.mouse.wheel(-30, 0);
  await expect(row).toHaveClass(/unread/);
});
