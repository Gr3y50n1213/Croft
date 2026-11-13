// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import AxeBuilder from '@axe-core/playwright';
import { test, expect, type Page } from '@playwright/test';
import { Companion, totp, virtualAuthenticator, watchForErrors } from './companion.ts';

let companion: Companion;

test.beforeAll(async () => {
  companion = await Companion.start();
});

test.afterAll(() => companion.stop());

/** Enrols a new user with the page's virtual authenticator: their TOTP secret and recovery codes. */
async function enrol(page: Page, name: string): Promise<{ secret: string; codes: string[] }> {
  await page.goto(companion.addUser(name));
  await page.getByRole('button', { name: /passkey/i }).click();
  await expect(page.locator('#step-backup')).toBeVisible();
  const secret = (await page.locator('#secret').textContent())!.trim();
  const codes = (await page.locator('#codes code').allTextContents()).map((c) => c.trim());
  return { secret, codes };
}

test('a passkey signs in to the app, the session survives a reload, and signing out ends it', async ({ page }) => {
  const errors = watchForErrors(page);
  await virtualAuthenticator(page);
  await enrol(page, 'ann');

  await page.goto(`${companion.url}/`);
  await expect(page).toHaveURL(/\/sign-in$/);
  // The sign-in screen has no accessibility violation (axe-core).
  await expect(page.getByRole('button', { name: 'Sign in with a passkey' })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations.map((v) => v.id)).toEqual([]);
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.getByTestId('user')).toHaveText('ann');
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();

  // The refresh token is a cookie no script can read; the page keeps nothing in storage.
  expect(await page.evaluate(() => document.cookie)).toBe('');
  expect(await page.evaluate(() => localStorage.length + sessionStorage.length)).toBe(0);
  const cookies = await page.context().cookies();
  const refresh = cookies.find((c) => c.name === 'croft_refresh');
  expect(refresh?.httpOnly).toBe(true);
  expect(refresh?.sameSite).toBe('Strict');

  // A reload: the access token is gone from memory; the cookie brings the session back.
  await page.reload();
  await expect(page.getByTestId('user')).toHaveText('ann');

  await page.getByRole('button', { name: 'Sign out' }).click();
  await expect(page).toHaveURL(/\/sign-in$/);
  expect((await page.context().cookies()).find((c) => c.name === 'croft_refresh')).toBeUndefined();
  await page.goto(`${companion.url}/`);
  await expect(page).toHaveURL(/\/sign-in$/);
  expect(errors).toEqual([]);
});

test('a recovery code leads to a new passkey, which then signs in', async ({ page }) => {
  const errors = watchForErrors(page);
  const lose = await virtualAuthenticator(page);
  const { secret, codes } = await enrol(page, 'bob');

  // The device with the passkey is lost; a new one signs in with TOTP and a recovery code.
  await lose();
  await virtualAuthenticator(page);
  await page.context().clearCookies();
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole('button', { name: 'Lost your passkey?' }).click();
  await page.getByLabel('User name').fill('bob');
  await page.getByLabel('Authenticator code').fill(totp(secret));
  await page.getByLabel('Recovery code').fill(codes[0]!);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page).toHaveURL(/\/passkey$/);
  await page.getByRole('button', { name: 'Make a passkey' }).click();
  await expect(page.getByRole('heading', { name: 'Passkey made' })).toBeVisible();
  await page.getByRole('link', { name: 'Sign in' }).click();
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.getByTestId('user')).toHaveText('bob');
  expect(errors).toEqual([]);
});

test('wrong recovery details say so in plain words', async ({ page }) => {
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole('button', { name: 'Lost your passkey?' }).click();
  await page.getByLabel('User name').fill('nobody');
  await page.getByLabel('Authenticator code').fill('000000');
  await page.getByLabel('Recovery code').fill('made-up-code');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveText(/didn't work/);
});

test('two tabs opening the app at once both stay signed in', async ({ page, context }) => {
  await virtualAuthenticator(page);
  await enrol(page, 'cat');
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.getByTestId('user')).toHaveText('cat');
  // Two new tabs: each has no access token yet and refreshes through the shared cookie at once.
  const [a, b] = await Promise.all([context.newPage(), context.newPage()]);
  await Promise.all([a.goto(`${companion.url}/`), b.goto(`${companion.url}/`)]);
  await expect(a.getByTestId('user')).toHaveText('cat');
  await expect(b.getByTestId('user')).toHaveText('cat');
  // And the session is still alive for a third.
  const c = await context.newPage();
  await c.goto(`${companion.url}/`);
  await expect(c.getByTestId('user')).toHaveText('cat');
});

test('opening the app signed out, again and again, never locks the browser out of signing in', async ({ page }) => {
  // A Companion of its own: the per-minute request limit is shared by the other tests' address.
  const own = await Companion.start();
  try {
    const errors = watchForErrors(page);
    await virtualAuthenticator(page);
    await page.goto(own.addUser('dan'));
    await page.getByRole('button', { name: /passkey/i }).click();
    await expect(page.locator('#step-backup')).toBeVisible();
    await page.context().clearCookies();
    // Twelve signed-out visits: before the fix each counted twice as a failed sign-in, and twenty
    // failures lock the address for 15 minutes.
    for (let i = 0; i < 12; i++) {
      await page.goto(`${own.url}/`);
      await expect(page).toHaveURL(/\/sign-in$/);
    }
    await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
    await expect(page.getByTestId('user')).toHaveText('dan');
    expect(errors).toEqual([]);
  } finally {
    own.stop();
  }
});
