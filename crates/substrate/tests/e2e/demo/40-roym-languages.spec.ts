import { expect, test, type Page } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import {
  addListingTranslation, addMessageLanguage, chooseHubLanguage, directoryAddedStatus, ensureOwnSynOrg,
  hubMessage, issueOwnCredential, loginWithDelegatedKey, openConversation, openMessages, pickCategory,
  publishListing, rpcCall, sendMessage, sendQuote, sendRequest, threadMessage,
} from '../hub-helpers';
import { newSession, requireEnv, step } from './steps';

const SCRIPT = '20-roym-setup.sh';

test.describe.configure({ mode: 'serial' });

// Story 20 has already deployed Roym, so this story needs no script. A new browser context starts
// in English, so the earlier stories are not affected, and the story can run again on the same
// nodes.
test('Roym languages story: Alice reads her Hub in Hindi, and it stays Hindi', async ({ page }) => {
  const env = readDemoEnv();
  const hub = requireEnv(env, 'ALICE_HUB_URL', SCRIPT);
  const key = requireEnv(env, 'ALICE_KEY', SCRIPT);
  const bobAddr = requireEnv(env, 'BOB_ADDR', SCRIPT);
  const aliceDirectoryDid = requireEnv(env, 'ALICE_DIRECTORY_DID', SCRIPT);
  const profileTab = page.locator('.tab-button').first();

  await step(page, "Alice's screen: sign in", async () => {
    await page.goto(hub);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page, key);
    await expect(profileTab).toHaveText(hubMessage('en', 'tab-profile'));
  });

  await step(page, "Alice's screen: switch the Hub to Hindi", async () => {
    await chooseHubLanguage(page, 'hi');
    await expect(profileTab).toHaveText(hubMessage('hi', 'tab-profile'));
  });

  await step(page, "Alice's screen: reload; the Hub is still in Hindi", async () => {
    // Not 'networkidle': a signed-in Hub keeps its event feed open, so the network is never idle.
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('lang', 'hi');
    // She is still signed in, and the tab names are still Hindi.
    await expect(page.locator('.session-bar')).toContainText('did:key:');
    await expect(profileTab).toHaveText(hubMessage('hi', 'tab-profile'));
  });

  await step(page, "Alice's screen: the Contacts screen is in Hindi too", async () => {
    await page.getByRole('button', { name: hubMessage('hi', 'tab-contacts') }).click();
    // The button of a screen, not only the tab names, is in Hindi.
    await expect(
      page.getByRole('button', { name: hubMessage('hi', 'contacts-upsert-button') }),
    ).toBeVisible();
    await expect(page.locator('.contacts-screen h2')).toHaveText(hubMessage('hi', 'contacts-title'));
  });

  await step(page, "Alice's screen: pick a category by its Hindi label in Listings", async () => {
    await page.getByRole('button', { name: hubMessage('hi', 'tab-listings') }).click();
    await pickCategory(page.locator('.listing-categories-input'), 'gardening', 'hi');
    const chip = page.locator('.listing-categories-input .picker-chip[data-code="gardening"]');
    await expect(chip).toContainText(hubMessage('hi', 'taxonomy-category-gardening'));
  });

  await step(page, "Alice's screen: add an English translation to her Hindi listing", async () => {
    await expect(page.locator('.listing-text-lang')).toHaveValue('hi');
    await addListingTranslation(page, 'en', 'Garden work', 'hi');
    await expect(page.locator('.listing-translation-title').last()).toHaveValue('Garden work');
  });

  await step(page, "Alice's screen: send Bob a chat message tagged with Hindi and English", async () => {
    await openMessages(page, 'hi');
    await openConversation(page, bobAddr, 'hi');
    const firstRowName = page.locator('.messages-language-row .messages-language-name').first();
    await expect(firstRowName).toHaveText('हिन्दी');

    await addMessageLanguage(page, 'en');
    await expect(page.locator('.messages-language-row')).toHaveCount(2);
    const runTag = Date.now().toString(36);
    const msg = `नमस्ते Bob, see you at the garden (${runTag})`;
    await sendMessage(page, msg, 'hi');

    const body = page.locator('.thread-messages .message .message-body', { hasText: msg }).first();
    await expect(body).toBeVisible({ timeout: 15_000 });
    await expect(body).toHaveAttribute('lang', 'hi');
  });

  await step(page, "Alice's screen: a search in Latin letters finds her Devanagari listing", async () => {
    // The tag keeps the story repeatable on the same nodes. It is base-36 digits and letters,
    // so it reads the same in both scripts.
    const runTag = Date.now().toString(36);
    const title = `नल वाला ${runTag}`;
    await ensureOwnSynOrg(page, aliceDirectoryDid, 'gardening', 'hi');
    await publishListing(page, title, 'gardening', aliceDirectoryDid, 'hi');

    await page.getByRole('button', { name: hubMessage('hi', 'tab-directory') }).click();
    await page.locator('.add-source-did').fill(aliceDirectoryDid);
    await page.getByRole('button', { name: hubMessage('hi', 'directory-add-button') }).click();
    await expect(page.locator('.add-source-status')).toHaveText(directoryAddedStatus('hi'), {
      timeout: 15_000,
    });

    await page.locator('.search-text-input').fill(`nal wala ${runTag}`);
    await page.getByRole('button', { name: hubMessage('hi', 'directory-search-button'), exact: true }).click();
    await expect(page.locator('.search-hit', { hasText: title })).toHaveCount(1, { timeout: 20_000 });
  });
});

// A message crosses from one node to the other on the sender's outbox tick, which the demo nodes
// set to one second. The wait is far longer, for a slow machine.
const DELIVERY_WAIT_MS = 60_000;

/** The state of the agreement of this run in one conversation, after the node has synced it. */
async function pairState(page: Page, conversation: string, runTag: string): Promise<string | undefined> {
  // A node files the other person's signed half inside `transaction.sync`, and `agreement.list`
  // only reads. So the sync is called here, and the check does not wait for the Hub to draw again.
  await rpcCall(page, 'transaction.sync', { conversation });
  const listed = await rpcCall(page, 'agreement.list', { conversation });
  const agreements = (listed.result?.agreements ?? []) as Array<{
    terms: { scope: string };
    pair: { state: string };
  }>;
  return agreements.find((a) => a.terms.scope.includes(runTag))?.pair.state;
}

/** The id of the conversation a node already has with an address. */
async function conversationWith(page: Page, address: string): Promise<string> {
  const opened = await rpcCall(page, 'conversation.open', { address });
  expect(opened.result?.conversation_id, 'conversation.open answers with the open thread').toBeTruthy();
  return opened.result.conversation_id as string;
}

test('Roym languages story: Alice, in Hindi, finds an English listing and accepts a quote from its owner', async ({ browser, page }, testInfo) => {
  const env = readDemoEnv();
  const need = (key: string) => requireEnv(env, key, SCRIPT);
  const alice = { hub: need('ALICE_HUB_URL'), key: need('ALICE_KEY'), addr: need('ALICE_ADDR') };
  const bob = {
    hub: need('BOB_HUB_URL'), key: need('BOB_KEY'), did: need('BOB_DID'), addr: need('BOB_ADDR'),
    directoryDid: need('BOB_DIRECTORY_DID'),
  };
  // A new tag in each text keeps an earlier run on the same nodes from matching.
  const runTag = Date.now().toString(36);
  const listingTitle = `Hedge trimming ${runTag}`;
  const requestText = `बगीचे की बाड़ काटनी है ${runTag}`;
  const quoteScope = `Trim the garden hedge ${runTag}`;
  const rupees = '₹1,00,000.50';

  const bobSession = await newSession(browser, testInfo);
  const bobPage = await bobSession.newPage();

  try {
    let listingId = '';

    await step(bobPage, "Bob's screen: sign in; his Hub stays in English", async () => {
      await bobPage.goto(bob.hub);
      await bobPage.waitForLoadState('networkidle');
      await loginWithDelegatedKey(bobPage, bob.key);
    });

    await step(bobPage, "Bob's screen: publish an English listing about garden work", async () => {
      await ensureOwnSynOrg(bobPage, bob.directoryDid, 'gardening', 'en');
      await issueOwnCredential(bobPage, 'gardening');
      // The listing's conversation address is Bob's own, so a person who finds it can write to him.
      listingId = await publishListing(
        bobPage, listingTitle, 'gardening', bob.directoryDid, 'en', undefined, bob.addr,
      );
    });

    await step(page, "Alice's screen: sign in and switch the Hub to Hindi", async () => {
      await page.goto(alice.hub);
      await page.waitForLoadState('networkidle');
      await loginWithDelegatedKey(page, alice.key);
      await chooseHubLanguage(page, 'hi');
    });

    await step(page, "Alice's screen: find Bob's listing by the Hindi name of its category", async () => {
      await page.getByRole('button', { name: hubMessage('hi', 'tab-directory') }).click();
      await page.locator('.add-source-did').fill(bob.directoryDid);
      await page.getByRole('button', { name: hubMessage('hi', 'directory-add-button') }).click();
      await expect(page.locator('.add-source-status')).toHaveText(directoryAddedStatus('hi'), {
        timeout: 15_000,
      });

      await pickCategory(page.locator('.search-categories'), 'gardening', 'hi');
      const found = page.waitForResponse(
        (res) => res.url().includes('/rpc') && res.request().postData()?.includes('directory.find') === true,
      );
      await page.getByRole('button', { name: hubMessage('hi', 'directory-search-button'), exact: true }).click();
      const answer = (await (await found).json()) as { result?: { hits?: Array<{ listing_id: string }> } };
      expect((answer.result?.hits ?? []).map((h) => h.listing_id)).toContain(listingId);

      // Alice's own directory holds other gardening listings, so the hit is found by its id.
      const hit = page.locator(`.search-hit[data-listing-id="${listingId}"]`);
      await expect(hit).toHaveCount(1, { timeout: 20_000 });
      await expect(hit.locator('.hit-title')).toHaveText(listingTitle);
      await expect(hit.locator('.hit-title')).toHaveAttribute('lang', 'en');
      await expect(hit.locator('.hit-categories')).toContainText(hubMessage('hi', 'taxonomy-category-gardening'));
    });

    await step(page, "Alice's screen: ask how to reach the listing owner", async () => {
      const hit = page.locator(`.search-hit[data-listing-id="${listingId}"]`);
      await hit.locator('.hit-engage').click();
      await expect(hit.locator('.hit-engage-out')).toContainText(bob.addr, { timeout: 15_000 });
    });

    await step(page, "Alice's screen: send Bob a request in Hindi; it names the category only", async () => {
      await openMessages(page, 'hi');
      await openConversation(page, bob.addr, 'hi');
      await sendRequest(page, requestText, 'gardening', 'hi');
      await expect(threadMessage(page, requestText).locator('.card-request')).toBeVisible({ timeout: 15_000 });
    });

    await step(bobPage, "Bob's screen: read the request and send a quote in rupees", async () => {
      await openMessages(bobPage);
      await openConversation(bobPage, alice.addr);
      const request = threadMessage(bobPage, requestText);
      await expect(request).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(request.locator('.request-description')).toHaveAttribute('lang', 'hi');
      await sendQuote(bobPage, requestText, {
        scope: quoteScope, currency: 'INR', amount: '100000.50', payee: bob.did,
      });
    });

    await step(page, "Alice's screen: read the quote in Hindi, with the price in rupees", async () => {
      await page.bringToFront();
      const quote = threadMessage(page, quoteScope);
      await expect(quote).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(quote.locator('.quote-total')).toHaveText(
        `${hubMessage('hi', 'terms-total').split('{')[0]}${rupees}`,
      );
      await expect(quote.locator('.quote-scope [lang]')).toHaveAttribute('lang', 'en');
    });

    await step(page, "Alice's screen: accept the quote; her Hub shows the signed agreement", async () => {
      const quote = threadMessage(page, quoteScope);
      await quote.getByRole('button', { name: hubMessage('hi', 'messages-accept-button') }).click();
      // She read the original, so no warning about a machine translation comes first.
      await expect(page.locator('.accept-mt-box')).toHaveCount(0);
      await expect(page.locator('.thread-messages .card-agreement-receipt').first()).toBeVisible({
        timeout: 15_000,
      });
    });

    await step(bobPage, "Bob's screen: his Hub shows the agreement, and both nodes hold both halves", async () => {
      await bobPage.bringToFront();
      await expect(bobPage.locator('.thread-messages .card-agreement-receipt').first()).toBeVisible({
        timeout: DELIVERY_WAIT_MS,
      });
      const aliceConversation = await conversationWith(page, bob.addr);
      const bobConversation = await conversationWith(bobPage, alice.addr);
      await expect
        .poll(() => pairState(page, aliceConversation, runTag), { timeout: DELIVERY_WAIT_MS })
        .toBe('complete');
      await expect
        .poll(() => pairState(bobPage, bobConversation, runTag), { timeout: DELIVERY_WAIT_MS })
        .toBe('complete');
    });

    await step(page, "Alice's screen: her signed half says she read the English original", async () => {
      const aliceConversation = await conversationWith(page, bob.addr);
      const listed = await rpcCall(page, 'agreement.list', { conversation: aliceConversation });
      const agreement = (listed.result.agreements as Array<{
        terms: { scope: string };
        counterparty: { envelope: string };
      }>).find((a) => a.terms.scope.includes(runTag))!;
      const verdict = await rpcCall(page, 'agreement.verify', { envelope: agreement.counterparty.envelope });
      expect(verdict.result.verified).toBe(true);
      expect(verdict.result.payload.viewed_lang).toBe('en');
      expect(verdict.result.payload.viewed_via_mt).toBe(false);
    });
  } finally {
    await bobSession.close();
  }
});
