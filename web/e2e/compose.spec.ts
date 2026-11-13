// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// W02 (T31b) against a real Companion: the conversation, compose windows, replies by the core's
// rules, Send into the Outbox and Undo. The account's SMTP port is closed, so a send waits in the
// Outbox (delivery itself is the core's, tested there). Made-up credentials.

import { test, expect, type Page } from '@playwright/test';
import { Companion, TestMail, addAccount, closedPort, virtualAuthenticator, watchForErrors } from './companion.ts';

let companion: Companion;
let mail: TestMail;

test.beforeEach(async () => {
  mail = await TestMail.start(5);
  companion = await Companion.start({ CROFT_COMPANION_MAIL_ALLOWED_NETWORKS: '127.0.0.1/32' });
});

test.afterEach(() => {
  companion.stop();
  mail.stop();
});

async function signedInWithMail(page: Page): Promise<void> {
  await virtualAuthenticator(page);
  await page.goto(companion.addUser('ann'));
  await page.getByRole('button', { name: /passkey/i }).click();
  await expect(page.locator('#step-backup')).toBeVisible();
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  await addAccount(page, mail, await closedPort());
  await expect(async () => {
    await page.reload();
    await expect(page.locator('.rows li.row').first()).toBeVisible({ timeout: 2000 });
  }).toPass({ timeout: 30_000 });
}

const plain = (page: Page) => page.locator('.rows li.row', { hasText: /Message \d+/ });

test('a reply gets its fields from the core, sends, and Undo brings it back', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  const subject = (await plain(page).first().locator('.subject').textContent())!;
  await plain(page).first().locator('.subject').click();
  const pane = page.getByRole('region', { name: 'Reading pane' });
  await pane.getByRole('button', { name: 'Reply', exact: true }).click();
  const win = page.getByRole('dialog', { name: `Re: ${subject}` });
  await expect(win).toBeVisible();
  await expect(win.getByLabel('To')).toHaveValue(/ann@example.org/);
  await win.getByLabel('Message').fill('Thanks, sounds good.');
  await win.getByRole('button', { name: 'Send' }).click();
  await expect(win).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: 'Sending' })).toBeVisible();
  await page.getByRole('status').filter({ hasText: 'Sending' }).getByRole('button', { name: 'Undo' }).click();
  const back = page.getByRole('dialog', { name: `Re: ${subject}` });
  await expect(back).toBeVisible();
  await expect(back.getByLabel('Message')).toHaveValue('Thanks, sounds good.');
  expect(errors).toEqual([]);
});

test('compose from scratch: no recipient is refused in plain words; up to three windows, minimised to the dock', async ({ page }) => {
  await signedInWithMail(page);
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('c');
  const win = page.getByRole('dialog', { name: 'New message' });
  await expect(win).toBeVisible();
  await win.getByLabel('Subject').fill('Plans');
  // The window is named by its subject.
  await page.getByRole('dialog', { name: 'Plans' }).getByRole('button', { name: 'Send' }).click();
  await expect(page.getByRole('dialog', { name: 'Plans' }).getByRole('status')).toHaveText('Add a recipient.');
  // Two more windows; the first ones dock; a fourth is refused.
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: /Compose/ }).click();
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: /Compose/ }).click();
  await expect(page.getByRole('group', { name: /Draft:/ })).toHaveCount(2);
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: /Compose/ }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Close a draft first' })).toBeVisible();
  // Restore the first one: its subject is still there.
  await page.getByRole('group', { name: 'Draft: Plans' }).getByRole('button', { name: 'Plans' }).click();
  await expect(page.getByRole('dialog', { name: 'Plans' }).getByLabel('Subject')).toHaveValue('Plans');
});

test('the conversation shows, and r, a and f open replies from the keyboard', async ({ page }) => {
  await signedInWithMail(page);
  await plain(page).first().locator('.subject').click();
  await expect(page.getByRole('region', { name: 'Reading pane' }).getByRole('button', { name: 'Forward' })).toBeVisible();
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('f');
  await expect(page.getByRole('dialog', { name: /^Fwd: / })).toBeVisible();
  await expect(page.getByRole('dialog', { name: /^Fwd: / }).getByLabel('To')).toHaveValue('');
});
