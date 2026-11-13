// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// W02 (T31b) against a real Companion: the conversation, compose windows, replies by the core's
// rules, Send into the Outbox and Undo. The account's SMTP port is closed, so a send waits in the
// Outbox (delivery itself is the core's, tested there). Made-up credentials.

import AxeBuilder from '@axe-core/playwright';
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
  await win.getByRole('button', { name: 'Send', exact: true }).click();
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
  await page.getByRole('dialog', { name: 'Plans' }).getByRole('button', { name: 'Send', exact: true }).click();
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
  // W02 with a compose window open and two docked: no accessibility violation (axe-core).
  const axe = await new AxeBuilder({ page }).analyze();
  expect(axe.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`)).toEqual([]);
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

test('a message is scheduled, listed in Scheduled with its time, and cancelled back into compose', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedInWithMail(page);
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: /Compose/ }).click();
  const win = page.getByRole('dialog', { name: 'New message' });
  await win.getByLabel('To').fill('cy@example.org');
  await win.getByLabel('Message').fill('See you Saturday.');
  // The window is named by its subject from here on.
  await win.getByLabel('Subject').fill('Seed swap');
  await page.getByRole('dialog', { name: 'Seed swap' }).getByRole('button', { name: /Schedule send/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Schedule send' });
  await expect(dialog.getByRole('button', { name: /Tomorrow morning/ })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations.map((v) => v.id)).toEqual([]);
  // A time more than a year away is refused in plain words, before anything is sent.
  const nextYear = new Date(Date.now() + 400 * 24 * 3600 * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  await dialog
    .getByLabel('Date and time')
    .fill(`${nextYear.getFullYear()}-${p(nextYear.getMonth() + 1)}-${p(nextYear.getDate())}T09:00`);
  await dialog.getByRole('button', { name: 'Schedule', exact: true }).click();
  await expect(dialog.getByRole('alert')).toHaveText('Choose a time within a year.');
  await dialog.getByRole('button', { name: /Tomorrow morning/ }).click();
  await expect(page.getByRole('dialog', { name: 'Seed swap' })).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: 'Scheduled for Tomorrow, ' })).toBeVisible();

  // Listed in Scheduled with its time, after a reload too (it waits in the Companion).
  await page.reload();
  await page.getByRole('navigation', { name: 'Folders' }).getByRole('button', { name: 'Scheduled' }).click();
  const row = page.getByTestId('scheduled');
  await expect(row).toHaveCount(1);
  await expect(row).toContainText('Seed swap');
  await expect(row).toContainText('Scheduled for Tomorrow, ');
  await expect(row).toContainText('1 recipient');
  expect((await new AxeBuilder({ page }).analyze()).violations.map((v) => v.id)).toEqual([]);

  // Cancel: the draft comes back to compose, nothing waits.
  await row.getByRole('button', { name: 'Cancel' }).click();
  const back = page.getByRole('dialog', { name: 'Seed swap' });
  await expect(back).toBeVisible();
  await expect(back.getByLabel('Message')).toHaveValue('See you Saturday.');
  await expect(page.getByTestId('scheduled')).toHaveCount(0);
  await expect(page.getByText('Nothing scheduled.')).toBeVisible();
  expect(errors).toEqual([]);
});
