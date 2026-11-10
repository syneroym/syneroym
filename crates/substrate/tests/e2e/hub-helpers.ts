import * as fs from 'fs';
import * as path from 'path';
import { expect, type Locator, type Page } from './fixtures';

// Drive the real delegated-key login: hand the Hub the session-key.json that
// `roymctl session delegate` produced, exactly as a person would after running
// the command themselves. The main suite's global setup names the file in
// ROYM_SESSION_KEY_FILE; a story with several people passes each one's file.
export async function loginWithDelegatedKey(
  page: Page,
  keyFile: string | undefined = process.env.ROYM_SESSION_KEY_FILE,
) {
  await expect(page.locator('.login-picker h2')).toHaveText('Sign in');
  expect(keyFile, 'a session key file is needed: pass one, or set ROYM_SESSION_KEY_FILE').toBeTruthy();
  await page.locator('input[type="file"]').setInputFiles(keyFile!);
  await expect(page.locator('.session-bar')).toContainText('did:key:', { timeout: 15_000 });
  await expect(page.locator('.session-bar')).toContainText('delegated');
}

// One JSON-RPC method over the logged-in session's own bearer token, the
// same shape scenario 1's own whoami check uses -- for a setup step the
// Hub has no screen for.
export async function rpcCall(page: Page, method: string, params: unknown) {
  return page.evaluate(
    async ({ method, params }) => {
      const win = window as unknown as {
        RoymSession?: { authHeaders: () => Record<string, string> };
      };
      const headers = win.RoymSession?.authHeaders() ?? {};
      const res = await fetch('/rpc', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...headers },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      });
      return res.json();
    },
    { method, params },
  );
}

// The Hub steps below are the one place that knows the Hub's button names and
// class names. The main suite and the demo story both call them, so a change
// in the Hub needs one fix here.

/** Save the signed-in person's profile with a display name and conversation address. */
export async function saveProfile(page: Page, displayName: string, conversationAddress: string) {
  await page.getByRole('button', { name: 'Profile', exact: true }).click();
  const textInputs = page.locator('.profile-screen input[type="text"]');
  await textInputs.nth(0).fill(displayName);
  await textInputs.nth(1).fill(conversationAddress);
  await page.locator('.profile-screen button[type="submit"]').click();
  await expect(page.locator('.profile-screen p')).toContainText('Saved record rec_', { timeout: 15_000 });
}

/** Add a contact by person DID and conversation address. */
export async function addContact(page: Page, personDid: string, conversationAddress: string) {
  await page.getByRole('button', { name: 'Contacts' }).click();
  const inputs = page.locator('.contacts-screen input');
  await inputs.nth(0).fill(personDid);
  await inputs.nth(1).fill(conversationAddress);
  await page.getByRole('button', { name: 'Upsert Contact' }).click();
  await expect(page.locator('.contacts-screen')).toContainText(personDid, { timeout: 15_000 });
}

/** Open the Messages screen. The Hub fetches the list again each time it is opened. */
export async function openMessages(page: Page) {
  await page.getByRole('button', { name: 'Messages' }).click();
  await expect(page.locator('.messages-screen')).toBeVisible();
}

/** Start a conversation with an address and show its thread. */
export async function openConversation(page: Page, conversationAddress: string) {
  await page.locator('.open-conversation-input').fill(conversationAddress);
  await page.getByRole('button', { name: 'Open conversation' }).click();
  await expect(page.locator('.conversation-thread h3').first()).toBeVisible({ timeout: 15_000 });
  // The Hub draws the first conversation by itself when the screen opens, and then draws the
  // opened one again. The selected row shows that the second drawing has replaced the first,
  // so a message typed now is not lost with the old thread.
  await expect(page.locator('.conversation-item.selected')).toBeVisible({ timeout: 15_000 });
}

/** Send a message in the open thread. The box clears only when the send succeeds. */
export async function sendMessage(page: Page, body: string) {
  await page.locator('.compose-input').fill(body);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.compose-input')).toHaveValue('', { timeout: 15_000 });
}

/** The message in the open thread that holds this text. */
export function threadMessage(page: Page, text: string) {
  return page.locator('.thread-messages .message', { hasText: text });
}

/**
 * The text of one message in the Hub's string catalog for a language, read from the catalog
 * file itself. A check that uses it still passes when the wording of a message is edited.
 * Only for messages on one line.
 */
export function hubMessage(tag: string, id: string): string {
  const file = path.join(__dirname, '../../../roym_web/ui/src/i18n', `${tag}.ftl`);
  const line = fs
    .readFileSync(file, 'utf8')
    .split('\n')
    .find((candidate) => candidate.startsWith(`${id} = `));
  if (line === undefined) {
    const taxFile = path.join(__dirname, '../../../roym_web/ui/src/i18n/taxonomy', `${tag}.ftl`);
    if (fs.existsSync(taxFile)) {
      const taxLine = fs
        .readFileSync(taxFile, 'utf8')
        .split('\n')
        .find((candidate) => candidate.startsWith(`${id} = `));
      if (taxLine !== undefined) return taxLine.slice(`${id} = `.length).trim();
    }
    throw new Error(`message ${id} is not in ${file}`);
  }
  return line.slice(`${id} = `.length).trim();
}

/** Pick a category in a category picker component by its code and UI language. */
export async function pickCategory(scope: Locator, code: string, tag = 'en') {
  const label = hubMessage(tag, 'taxonomy-category-' + code);
  const filter = scope.locator('.picker-filter');
  await filter.fill(label);
  await scope.locator(`.picker-option[data-code="${code}"]`).click();
  await expect(scope.locator(`.picker-chip[data-code="${code}"]`)).toBeVisible();
}

/** Add a translation row in the listing editor and fill its language and title. */
export async function addListingTranslation(page: Page, lang: string, title: string, tag = 'en') {
  const addBtnText = hubMessage(tag, 'listing-translation-add');
  await page.getByRole('button', { name: addBtnText, exact: true }).click();
  await page.locator('.listing-translation-lang').last().selectOption(lang);
  await page.locator('.listing-translation-title').last().fill(title);
}

/** Pick a language in the Hub's language list. The page loads again in that language. */
export async function chooseHubLanguage(page: Page, tag: string) {
  await page.locator('.language-picker').selectOption(tag);
  await expect(page.locator('html')).toHaveAttribute('lang', tag, { timeout: 15_000 });
}
