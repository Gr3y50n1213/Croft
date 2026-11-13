// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// W04 (T32a) against a real Companion. Passkeys are Chromium's virtual authenticators (made-up keys,
// this test only); the mail server is the core's IMAP test server.

import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import {
  Companion,
  TestMail,
  addAccount,
  copyPasskeys,
  virtualAuthenticator,
  watchForErrors,
} from "./companion.ts";

let companion: Companion;
let mail: TestMail | null = null;

// Without the service worker (T32b): one test blocks the app's scripts to show that theme.js alone
// paints the look, and the worker would answer them from its cache. The worker has its own tests.
test.use({ serviceWorkers: "block" });

test.beforeEach(async () => {
  companion = await Companion.start({
    CROFT_COMPANION_MAIL_ALLOWED_NETWORKS: "127.0.0.1/32",
  });
});

test.afterEach(() => {
  companion.stop();
  mail?.stop();
  mail = null;
});

let users = 0;

/** A new user, enrolled and signed in (W01 shown); the function that removes their authenticator. */
async function signedIn(page: Page): Promise<() => Promise<void>> {
  const remove = await virtualAuthenticator(page);
  await page.goto(companion.addUser(`user${++users}`));
  await page.getByRole("button", { name: /passkey/i }).click();
  await expect(page.locator("#step-backup")).toBeVisible();
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole("button", { name: "Sign in with a passkey" }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  return remove;
}

const html = (page: Page, name: string) =>
  page.locator("html").getAttribute(name);

test("W04 opens from the inbox, every category shows and passes axe", async ({
  page,
}) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  await page.getByRole("link", { name: "Settings" }).click();
  await expect(page.locator('[data-screen="W04"]')).toBeVisible();
  const nav = page.getByRole("navigation", { name: "Settings" });
  for (const [name, heading] of [
    ["Accounts", "Accounts"],
    ["Swipe and hover actions", "Swipe and hover actions"],
    ["Appearance", "Appearance"],
    ["Security", "Security"],
    ["About", "About"],
  ]) {
    await nav.getByRole("link", { name: new RegExp(`^${name}`) }).click();
    await expect(
      page.getByRole("heading", { level: 2, name: heading }),
    ).toBeVisible();
    await expect(
      nav.getByRole("link", { name: new RegExp(`^${name}`) }),
    ).toHaveAttribute("aria-current", "page");
    await page.waitForLoadState("networkidle");
    const axe = await new AxeBuilder({ page }).analyze();
    expect(
      axe.violations.map(
        (v) => `${name}: ${v.id} ${v.nodes.map((n) => n.target).join(" ")}`,
      ),
    ).toEqual([]);
  }
  await expect(page.getByTestId("version")).toHaveText(/^\d+\.\d+\.\d+/);
  await nav.getByRole("link", { name: /^Accounts/ }).click();
  await expect(page.getByText("No mail accounts yet.")).toBeVisible();
  await page.getByRole("link", { name: "Back to mail" }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  expect(errors).toEqual([]);
});

test("the look changes at once, survives a reload without the app, and passes axe in dark high contrast", async ({
  page,
}) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  await page.goto(`${companion.url}/settings/appearance`);
  await expect(html(page, "data-croft-mode")).resolves.toBe("light");
  await page.getByRole("radio", { name: "Dark", exact: true }).check();
  await page.getByRole("radio", { name: "Tide" }).check();
  await page.getByRole("radio", { name: "High" }).check();
  await page.getByRole("radio", { name: "Compact" }).check();
  await page.getByRole("radio", { name: "Round" }).check();
  await page.getByRole("radio", { name: "Flat" }).check();
  await page.getByRole("checkbox", { name: /Pure black/ }).check();
  const expected = {
    "data-croft-mode": "pure-black",
    "data-croft-theme": "tide",
    "data-croft-contrast": "high",
    "data-croft-density": "compact",
    "data-croft-roundness": "round",
    "data-croft-list": "flat",
  };
  for (const [k, v] of Object.entries(expected))
    await expect(page.locator("html")).toHaveAttribute(k, v);
  // The page's colours are the palette's (not a hard-coded default).
  const bg = await page.evaluate(
    () => getComputedStyle(document.documentElement).backgroundColor,
  );
  expect(bg).toBe("rgb(0, 0, 0)");
  const axe = await new AxeBuilder({ page }).analyze();
  expect(axe.violations.map((v) => v.id)).toEqual([]);

  // Reloaded with the app's scripts blocked: theme.js alone paints the chosen look, so there is no
  // flash of the default look before the app starts.
  await page.route("**/_app/**", (r) => r.abort());
  await page.reload();
  for (const [k, v] of Object.entries(expected))
    await expect(page.locator("html")).toHaveAttribute(k, v);
  await page.unroute("**/_app/**");
  // The blocked start-up script is the one error expected.
  expect(errors).toEqual([
    expect.stringContaining("Failed to fetch dynamically imported module"),
  ]);
  errors.length = 0;

  // Light again: pure black no longer applies, and the choice is kept in another tab too.
  await page.reload();
  await page.getByRole("radio", { name: "Light", exact: true }).check();
  await expect(page.locator("html")).toHaveAttribute(
    "data-croft-mode",
    "light",
  );
  await expect(
    page.getByRole("checkbox", { name: /Pure black/ }),
  ).toBeDisabled();
  const other = await page.context().newPage();
  await other.goto(`${companion.url}/settings/appearance`);
  await expect(other.locator("html")).toHaveAttribute(
    "data-croft-mode",
    "light",
  );
  await page.getByRole("radio", { name: "Moss" }).check();
  await expect(other.locator("html")).toHaveAttribute(
    "data-croft-theme",
    "moss",
  );
  expect(errors).toEqual([]);
});

test("the swipe setting changes the hover buttons in W01", async ({ page }) => {
  const errors = watchForErrors(page);
  mail = await TestMail.start(3);
  await signedIn(page);
  await addAccount(page, mail);
  await page.goto(`${companion.url}/settings/swipe`);
  await page
    .getByRole("combobox", { name: "Swipe right" })
    .selectOption("star");
  await page.getByRole("combobox", { name: "Swipe left" }).selectOption("spam");
  // No preset matches now.
  await expect(page.getByRole("radio", { name: /Triage/ })).not.toBeChecked();
  await page.goto(`${companion.url}/settings/accounts`);
  await expect(page.getByTestId("account")).toHaveCount(1);
  await page.getByRole("link", { name: "Back to mail" }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  await expect(async () => {
    await page.reload();
    await expect(page.locator(".rows li.row").first()).toBeVisible({
      timeout: 2000,
    });
  }).toPass({ timeout: 30_000 });
  const row = page.locator(".rows li.row").first();
  await row.hover();
  const quick = row.locator(".quick button");
  await expect(quick.nth(0)).toHaveAccessibleName(/^Star/);
  await expect(quick.nth(1)).toHaveAccessibleName(/^Spam/);
  // Back to the default (Triage): Mark read or unread, then Delete.
  await page.goto(`${companion.url}/settings/swipe`);
  await page.getByRole("button", { name: "Reset to default" }).click();
  await expect(page.getByRole("radio", { name: /Triage/ })).toBeChecked();
  await page.goto(companion.url);
  await page.locator(".rows li.row").first().hover();
  await expect(
    page.locator(".rows li.row").first().locator(".quick button").nth(0),
  ).toHaveAccessibleName(/^Mark read or unread/);
  expect(errors).toEqual([]);
});

test("a passkey is added and removed, never the last one", async ({ page }) => {
  const errors = watchForErrors(page);
  const removeFirst = await signedIn(page);
  await page.goto(`${companion.url}/settings/security`);
  const keys = page.getByTestId("passkey");
  await expect(keys).toHaveCount(1);
  await expect(keys.getByRole("button", { name: "Remove" })).toBeDisabled();
  // A second device: a new authenticator (the first one would refuse a second passkey for the same
  // account).
  await removeFirst();
  await virtualAuthenticator(page);
  await page
    .getByRole("button", { name: "Add a passkey on this device" })
    .click();
  await expect(page.getByRole("status")).toHaveText("Passkey added.");
  await expect(keys).toHaveCount(2);
  await keys.first().getByRole("button", { name: "Remove" }).click();
  await page
    .getByRole("dialog", { name: "Remove this passkey?" })
    .getByRole("button", { name: "Remove" })
    .click();
  await expect(page.getByRole("status")).toHaveText("Passkey removed.");
  await expect(keys).toHaveCount(1);
  await expect(keys.getByRole("button", { name: "Remove" })).toBeDisabled();
  expect(errors).toEqual([]);
});

test("another browser is signed out from W04", async ({ page, browser }) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  // The same person in a second browser (its own cookies), with a copy of the passkey.
  const second = await (await browser.newContext()).newPage();
  await virtualAuthenticator(second);
  await copyPasskeys(page, second);
  await second.goto(`${companion.url}/sign-in`);
  await second.getByRole("button", { name: "Sign in with a passkey" }).click();
  await expect(second.locator('[data-screen="W01"]')).toBeVisible();

  await page.goto(`${companion.url}/settings/security`);
  const devices = page.getByTestId("session");
  await expect(devices).toHaveCount(2);
  await expect(devices.first()).toContainText("This browser");
  await expect(
    devices.first().getByRole("button", { name: "Sign out" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Sign out everywhere else" }).click();
  await expect(page.getByRole("status")).toHaveText(
    "Signed out everywhere else.",
  );
  await expect(devices).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Sign out everywhere else" }),
  ).toHaveCount(0);
  // The second browser's session is over: a reload asks it to sign in.
  await second.reload();
  await expect(second).toHaveURL(/\/sign-in$/);
  await second.context().close();
  expect(errors).toEqual([]);
});

test('"Delete my Companion data" deletes the account after typing its name (T34e)', async ({
  page,
}) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  const name = `user${users}`;
  // The link the phone app and the store listing give: straight to Security when signed in.
  await page.goto(`${companion.url}/delete`);
  await expect(page).toHaveURL(/\/settings\/security$/);
  await page.getByTestId("delete-open").click();
  const dialog = page.getByRole("dialog", {
    name: "Delete your Companion data?",
  });
  await expect(dialog.getByTestId("delete-confirm")).toBeDisabled();
  await dialog.getByTestId("delete-name").fill("someone-else");
  await dialog.getByTestId("delete-confirm").click();
  await expect(dialog.getByRole("alert")).toHaveText(
    "That is not your user name.",
  );
  await dialog.getByTestId("delete-name").fill(name);
  await dialog.getByTestId("delete-confirm").click();
  await expect(page.getByTestId("deleted")).toBeVisible();
  // Signed out for good: the app goes back to sign-in, and the passkey no longer signs in.
  await page.goto(`${companion.url}/`);
  await expect(page).toHaveURL(/\/sign-in$/);
  await page.getByRole("button", { name: "Sign in with a passkey" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page).toHaveURL(/\/sign-in$/);
  expect(errors.filter((e) => !/401|404|invalid/i.test(e))).toEqual([]);
});
