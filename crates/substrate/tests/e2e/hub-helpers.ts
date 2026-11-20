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
  await page.locator('.contacts-screen .contacts-did-input').fill(personDid);
  await page.locator('.contacts-screen .contacts-address-input').fill(conversationAddress);
  await page.locator('.contacts-screen .contacts-upsert-button').click();
  await expect(page.locator('.contacts-screen')).toContainText(personDid, { timeout: 15_000 });
}

/**
 * Grant the signed-in person a membership credential for a category from the SynOrg on their own
 * node. A publish needs one. The Hub has no screen for issuing one yet, so this calls the node
 * directly, over the wire the test already authenticates with.
 */
export async function issueOwnCredential(page: Page, category: string) {
  const whoami = await rpcCall(page, 'session.whoami', {});
  const issued = await rpcCall(page, 'credential.issue', {
    member_did: whoami.result?.did,
    categories: [category],
    expires_at_secs: Math.floor(Date.now() / 1000) + 365 * 24 * 3600,
  });
  expect(issued.result?.record_id).toBeTruthy();
}

/** Ensure the signed-in person's node runs a SynOrg with generous limits. */
export async function ensureOwnSynOrg(
  page: Page,
  directoryDid?: string,
  category = 'bicycle-repair',
  lang = 'en',
) {
  await page.getByRole('button', { name: hubMessage(lang, 'tab-synorg'), exact: true }).click();
  const status = await page.locator('.synorg-status').innerText();
  if (status.includes(hubMessage(lang, 'synorg-status-none'))) {
    await page.locator('.synorg-name').fill('E2E Trades');
    await page.locator('.synorg-rules').fill('Be honest. Show up.');
    await pickCategory(page.locator('.synorg-categories'), category, lang);
    await page.locator('.synorg-support').fill('help@example.org');
    await page.locator('.synorg-dispute').fill('Email support.');
    await page.locator('.synorg-retention-days').fill('30');
    await page.getByRole('button', { name: hubMessage(lang, 'synorg-save-button') }).click();
    await expect(page.locator('.synorg-save-status')).toHaveText(hubMessage(lang, 'common-saved'), { timeout: 15_000 });
    await page.getByRole('button', { name: hubMessage(lang, 'tab-synorg'), exact: true }).click();
  } else {
    const hasCategory = (await page.locator(`.synorg-categories .picker-chip[data-code="${category}"]`).count()) > 0;
    if (!hasCategory) {
      await pickCategory(page.locator('.synorg-categories'), category, lang);
      await page.getByRole('button', { name: hubMessage(lang, 'synorg-save-button') }).click();
      await expect(page.locator('.synorg-save-status')).toHaveText(hubMessage(lang, 'common-saved'), { timeout: 15_000 });
      await page.getByRole('button', { name: hubMessage(lang, 'tab-synorg'), exact: true }).click();
    }
  }
  // Every test shares this one node, so publications from earlier tests
  // sit in the 24 h ledger. Keep the limit generous so a fresh publish
  // does not hit it -- the one case that tests the limit lowers it again.
  await expect(page.locator('.limit-max')).toBeVisible({ timeout: 15_000 });
  await page.locator('.limit-window').fill('86400');
  await page.locator('.limit-max').fill('1000'); // the ceiling; 0 would block every publish
  await page.getByRole('button', { name: hubMessage(lang, 'synorg-limit-save') }).click();
  await expect(page.locator('.limit-status')).toHaveText(hubMessage(lang, 'common-saved'), { timeout: 15_000 });

  // A publish needs a credential: always ensure one is issued for the category.
  await issueOwnCredential(page, category);
}

/**
 * Publish a listing to the directory under a SynOrg, and return its id. The listing's
 * conversation address is `conversationAddress`; it defaults to the directory DID, which is
 * enough when no test opens a chat from the listing.
 */
export async function publishListing(
  page: Page,
  title: string,
  category = 'bicycle-repair',
  directoryDid = '',
  lang = 'en',
  textLang?: string,
  conversationAddress = directoryDid,
): Promise<string> {
  await page.getByRole('button', { name: hubMessage(lang, 'tab-listings') }).click();
  // The text language of the listing. Without it the select keeps the Hub's own language.
  if (textLang) await page.locator('.listing-text-lang').selectOption(textLang);
  await page.locator('.listing-title-input').fill(title);
  await pickCategory(page.locator('.listing-categories-input'), category, lang);
  await page.locator('.listing-address-input').fill(conversationAddress);
  await page.locator('.block-payment .block-enabled').check();
  await page.locator('.payment-amount-input').fill('40');
  await page.locator('.save-listing').click();
  await expect(page.locator('.listing-save-result')).toContainText('lst_', { timeout: 15_000 });
  // The id is `lst_` and the z-base-32 digest of the issuer and the slug.
  const saved = await page.locator('.listing-save-result').innerText();
  const listingId = saved.match(/lst_[a-z0-9]+/)?.[0];
  expect(listingId, 'the save result shows the listing id').toBeTruthy();

  const row = page.locator('.listing-row', { hasText: title }).first();
  await row.locator('.publish-directory-did').fill(directoryDid);
  await row.locator('.publish-listing').click();
  await expect(row.locator('.publish-status')).toContainText(hubMessage(lang, 'listing-published'), { timeout: 15_000 });
  return listingId!;
}

/** Open the Messages screen. The Hub fetches the list again each time it is opened. */
export async function openMessages(page: Page, tag = 'en') {
  // Exact: the Hindi tab name is also part of the "Message this listing owner" button of a hit.
  await page.getByRole('button', { name: hubMessage(tag, 'tab-messages'), exact: true }).click();
  await expect(page.locator('.messages-screen')).toBeVisible();
}

/** Start a conversation with an address and show its thread. */
export async function openConversation(page: Page, conversationAddress: string, tag = 'en') {
  const openButton = page.getByRole('button', { name: hubMessage(tag, 'messages-open-button') });
  await page.locator('.open-conversation-input').fill(conversationAddress);
  await openButton.click();
  await expect(page.locator('.conversation-thread h3').first()).toBeVisible({ timeout: 15_000 });
  // The Hub draws the first conversation by itself when the screen opens, and then draws the
  // opened one again. The click turns the button off and the Hub turns it on again when the work
  // of the click is over, so no drawing of the click is still pending and a form opened now is
  // not lost with a thread that is about to be replaced. A selected row does not show this: a
  // feed event can select the row of the first conversation before the second drawing starts.
  // (When a feed event overtakes the click's list refill, the Hub draws no second thread at all;
  // see the deferred backlog.)
  await expect(openButton).toBeEnabled({ timeout: 15_000 });
  await expect(page.locator('.open-error')).toHaveText('');
}

/** Send a message in the open thread. The box clears only when the send succeeds. */
export async function sendMessage(page: Page, body: string, tag = 'en') {
  await page.locator('.compose-input').fill(body);
  await page.getByRole('button', { name: hubMessage(tag, 'messages-send-button'), exact: true }).click();
  await expect(page.locator('.compose-input')).toHaveValue('', { timeout: 15_000 });
}

/** Pick a language in the message composer's language list. */
export async function addMessageLanguage(page: Page, lang: string) {
  await page.locator('.messages-languages-add').selectOption(lang);
}

/** The message in the open thread that holds this text. */
export function threadMessage(page: Page, text: string) {
  return page.locator('.thread-messages .message', { hasText: text });
}

/** The id of the conversation a node already has with an address. */
export async function conversationWith(page: Page, address: string): Promise<string> {
  const opened = await rpcCall(page, 'conversation.open', { address });
  expect(opened.result?.conversation_id, 'conversation.open answers with the open thread').toBeTruthy();
  return opened.result.conversation_id as string;
}

/** What agreement.list says about one agreement. */
export interface AgreementSummary {
  quote_record_id: string;
  terms: { scope: string | { text?: string } };
  pair: { state: string };
  commitments: Array<{ commitment_id: string; state: string }>;
}

/** Reads the agreement of this run in one conversation from agreement.list. The scope holds the run tag. */
export async function fetchAgreement(
  page: Page,
  conversation: string,
  runTag: string,
): Promise<AgreementSummary | undefined> {
  await rpcCall(page, 'transaction.sync', { conversation });
  const listed = await rpcCall(page, 'agreement.list', { conversation });
  const agreements = (listed.result?.agreements ?? []) as AgreementSummary[];
  return agreements.find((a) => {
    const scopeStr = typeof a.terms.scope === 'object' && a.terms.scope !== null ? a.terms.scope.text : a.terms.scope;
    return typeof scopeStr === 'string' && scopeStr.includes(runTag);
  });
}

/**
 * Send a request in the open thread: a description and one category. `tag` is the Hub language,
 * which names the category in the picker. The request form closes when the send works.
 */
export async function sendRequest(page: Page, description: string, category: string, tag = 'en') {
  await page.locator('.toggle-request-form').click();
  await expect(page.locator('.request-form')).toBeVisible();
  await page.locator('.request-desc-input').fill(description);
  await pickCategory(page.locator('.request-cats-input'), category, tag);
  await page.locator('.send-request-button').click();
  await expect(page.locator('.request-form')).not.toBeVisible({ timeout: 15_000 });
}

/**
 * Send a request from the already-open pre-filled form: description only.
 * The request form closes when the send works.
 */
export async function sendRequestFromForm(page: Page, description: string) {
  await page.locator('.request-desc-input').fill(description);
  await page.locator('button.send-request-button').click();
  await expect(page.locator('.request-form')).not.toBeVisible({ timeout: 15_000 });
}

/**
 * In the directory, add a source, search for a category, and return the hit matching the listing id.
 */
export async function findListingHit(
  page: Page,
  directoryDid: string,
  category: string,
  listingId: string,
  listingTitle: string,
  lang = 'en',
): Promise<Locator> {
  await page.bringToFront();
  await page.getByRole('button', { name: hubMessage(lang, 'tab-directory'), exact: true }).click();
  await page.locator('.add-source-did').fill(directoryDid);
  await page.getByRole('button', { name: hubMessage(lang, 'directory-add-button') }).click();
  await expect(page.locator('.add-source-status')).toHaveText(directoryAddedStatus(lang), {
    timeout: 15_000,
  });

  await pickCategory(page.locator('.search-categories'), category, lang);
  const found = page.waitForResponse(
    (res) => res.url().includes('/rpc') && res.request().postData()?.includes('directory.find') === true,
  );
  await page.getByRole('button', { name: hubMessage(lang, 'directory-search-button'), exact: true }).click();
  const answer = (await (await found).json()) as { result?: { hits?: Array<{ listing_id: string }> } };
  expect((answer.result?.hits ?? []).map((h) => h.listing_id)).toContain(listingId);

  const hit = page.locator(`.search-hit[data-listing-id="${listingId}"]`);
  await expect(hit).toHaveCount(1, { timeout: 20_000 });
  await expect(hit.locator('.hit-title')).toHaveText(listingTitle);
  return hit;
}

/**
 * Tap "Ask" on a directory hit, assert navigation to Messages, and assert the request form
 * has the listing id and category pre-filled with an empty description.
 */
export async function askFromHit(
  page: Page,
  hit: Locator,
  listingId: string,
  category: string,
): Promise<void> {
  await hit.locator('.hit-engage').click();
  await expect(page.locator('.messages-screen')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.request-form')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.request-listing-input')).toHaveValue(listingId);
  await expect(page.locator(`.request-cats-input .picker-chip[data-code="${category}"]`)).toBeVisible();
  await expect(page.locator('.request-desc-input')).toHaveValue('');
}

/**
 * Answer the request card that holds `requestText` with a quote that is carried out at the
 * giver's place, so the form asks for no address. The quote form closes when the send works.
 */
export async function sendQuote(
  page: Page,
  requestText: string,
  quote: { scope: string; currency: string; amount: string; payee: string; give?: string },
) {
  const card = threadMessage(page, requestText).first();
  await card.locator('.quote-request-button').click();
  const form = page.locator('.quote-form');
  await expect(form).toBeVisible();
  await form.locator('.quote-scope-input').fill(quote.scope);
  if (quote.give) {
    await form.locator('.quote-give-input').fill(quote.give);
  }
  await form.locator('.quote-currency-input').fill(quote.currency);
  await form.locator('.quote-amount-input').fill(quote.amount);
  await form.locator('.quote-payee-input').fill(quote.payee);
  await form.locator('.quote-where-select').selectOption('at-giver');
  await expect(form.locator('.quote-size-line')).toBeVisible();
  await expect(form.locator('.quote-size-line')).toContainText('of 96 KiB');
  await form.locator('.submit-quote-button').click();
  await expect(form).toHaveCount(0, { timeout: 15_000 });
}

/**
 * The text of the status line after "Add directory" succeeds, in the Hub language `tag`: `Added`,
 * `Added` with the probe note, or `Added` with the silent note. The note has an argument, so only
 * the text before the first `{` is known.
 */
export function directoryAddedStatus(tag: string): RegExp {
  const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const note = hubMessage(tag, 'directory-added-note').split('{')[0].trim();
  const alternatives = [
    hubMessage(tag, 'directory-added'),
    hubMessage(tag, 'directory-added-silent'),
    note,
  ];
  return new RegExp(`^(?:${alternatives.map(escape).join('|')})`);
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
