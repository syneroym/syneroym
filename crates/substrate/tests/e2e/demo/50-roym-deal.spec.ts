import { expect, test, type Locator, type Page } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import {
  askFromHit, conversationWith, ensureOwnSynOrg, fetchAgreement, findListingHit,
  loginWithDelegatedKey, openConversation, openMessages, publishListing,
  sendQuote, sendRequest, sendRequestFromForm, threadMessage,
} from '../hub-helpers';
import { DELIVERY_WAIT_MS, newSession, requireEnv, step } from './steps';

const SCRIPT = '20-roym-setup.sh';

test.describe.configure({ mode: 'serial' });

/** The receipt cards of this run in the open thread. The scope on a card holds the run tag. */
function runReceiptCard(page: Page, runTag: string): Locator {
  return page.locator('.thread-messages .card-agreement-receipt', { hasText: runTag });
}

test('Roym deal story: Alice and Bob form an agreement and decline a quote with a note', async ({ browser, page }, testInfo) => {
  const env = readDemoEnv();
  const need = (key: string) => requireEnv(env, key, SCRIPT);
  const alice = { hub: need('ALICE_HUB_URL'), key: need('ALICE_KEY'), did: need('ALICE_DID'), addr: need('ALICE_ADDR') };
  const bob = {
    hub: need('BOB_HUB_URL'), key: need('BOB_KEY'), did: need('BOB_DID'), addr: need('BOB_ADDR'),
    directoryDid: need('BOB_DIRECTORY_DID'),
  };

  const runTag = Date.now().toString(36);
  const listingTitle = `Hindi lessons (${runTag})`;
  let listingId = '';
  const enquiryText = `Hindi lessons for my son (${runTag})`;
  const quoteScope = `Hindi lessons (${runTag})`;
  const giveText = 'One lesson, one hour';

  const declineEnquiryText = `Advanced Hindi (${runTag})`;
  const declineQuoteScope = `Advanced lessons (${runTag})`;
  const declineNoteText = 'Prefer beginner level for now';

  const bobSession = await newSession(browser, testInfo);
  const bobPage = await bobSession.newPage();

  try {
    await step(page, "Alice's screen: sign in", async () => {
      await page.goto(alice.hub);
      await page.waitForLoadState('networkidle');
      await loginWithDelegatedKey(page, alice.key);
    });

    await step(bobPage, "Bob's screen: sign in", async () => {
      await bobPage.goto(bob.hub);
      await bobPage.waitForLoadState('networkidle');
      await loginWithDelegatedKey(bobPage, bob.key);
    });

    await step(bobPage, "Bob's screen: publish a listing for Hindi lessons", async () => {
      await ensureOwnSynOrg(bobPage, bob.directoryDid, 'language-lessons', 'en');
      listingId = await publishListing(
        bobPage, listingTitle, 'language-lessons', bob.directoryDid, 'en', undefined, bob.addr,
      );
    });

    await step(page, "Alice's screen: find Bob's listing in the directory and tap Ask", async () => {
      const hit = await findListingHit(page, bob.directoryDid, 'language-lessons', listingId, listingTitle, 'en');
      await askFromHit(page, hit, listingId, 'language-lessons');
    });

    await step(page, "Alice's screen: send Bob an enquiry from the pre-filled form", async () => {
      await sendRequestFromForm(page, enquiryText);
    });

    await step(bobPage, "Bob's screen: read enquiry and send a quote giving one lesson for 100 INR", async () => {
      await openMessages(bobPage);
      await openConversation(bobPage, alice.addr);
      const enquiry = threadMessage(bobPage, enquiryText);
      await expect(enquiry).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(enquiry.locator('.request-listing')).toContainText(listingId);
      await sendQuote(bobPage, enquiryText, {
        scope: quoteScope,
        give: giveText,
        currency: 'INR',
        amount: '100',
        payee: bob.did,
      });
    });

    await step(page, "Alice's screen: quote shows both commitments and Alice accepts", async () => {
      await page.bringToFront();
      const quote = threadMessage(page, quoteScope);
      await expect(quote).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(quote.locator('.quote-commitment').first()).toContainText('One lesson, one hour');
      await expect(quote.locator('.quote-commitment').nth(1)).toContainText('You give:');
      await expect(quote).toContainText('₹100');

      await quote.getByRole('button', { name: 'Accept' }).click();
      await expect(page.locator('.accept-mt-box')).toHaveCount(0);
      await expect(runReceiptCard(page, runTag).first()).toBeVisible({ timeout: 15_000 });
    });

    await step(bobPage, "Bob's screen: agreement receipt is visible on both Hubs", async () => {
      await bobPage.bringToFront();
      await expect(runReceiptCard(bobPage, runTag).first()).toBeVisible({ timeout: DELIVERY_WAIT_MS });
    });

    await step(page, "Both Hubs show the agreement with two promised commitments", async () => {
      const aliceConversation = await conversationWith(page, bob.addr);
      const bobConversation = await conversationWith(bobPage, alice.addr);

      await expect
        .poll(async () => {
          const agr = await fetchAgreement(page, aliceConversation, runTag);
          return agr?.commitments.map((c) => c.state);
        }, { timeout: DELIVERY_WAIT_MS })
        .toEqual(['promised', 'promised']);

      await expect
        .poll(async () => {
          const agr = await fetchAgreement(bobPage, bobConversation, runTag);
          return agr?.commitments.map((c) => c.state);
        }, { timeout: DELIVERY_WAIT_MS })
        .toEqual(['promised', 'promised']);
    });

    await step(page, "Alice's screen: send Bob an enquiry for advanced lessons", async () => {
      await page.bringToFront();
      await sendRequest(page, declineEnquiryText, 'language-lessons');
      await expect(threadMessage(page, declineEnquiryText).locator('.card-enquiry')).toBeVisible({ timeout: 15_000 });
    });

    await step(bobPage, "Bob's screen: send a quote for advanced lessons", async () => {
      await bobPage.bringToFront();
      const enquiry = threadMessage(bobPage, declineEnquiryText);
      await expect(enquiry).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await sendQuote(bobPage, declineEnquiryText, {
        scope: declineQuoteScope,
        give: 'Advanced literature, one hour',
        currency: 'INR',
        amount: '200',
        payee: bob.did,
      });
    });

    await step(page, "Alice's screen: decline the quote with a note", async () => {
      await page.bringToFront();
      const quote = threadMessage(page, declineQuoteScope);
      await expect(quote).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await quote.locator('.decline-quote-button').click();
      const dialog = page.locator('.decline-dialog');
      await expect(dialog).toBeVisible();
      await dialog.locator('.decline-note-input').fill(declineNoteText);
      await dialog.locator('button.confirm-decline').click();
      await expect(dialog).toHaveCount(0, { timeout: 15_000 });
      await expect(quote.locator('.accept-quote-button')).toHaveCount(0);
      await expect(quote.locator('.decline-quote-button')).toHaveCount(0);
    });

    await step(bobPage, "Bob's screen: see the quote declined with the note", async () => {
      await bobPage.bringToFront();
      const declineCard = bobPage.locator('.thread-messages .card-deal-decline', { hasText: declineNoteText });
      await expect(declineCard.first()).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(declineCard.first().locator('.decline-reason')).toHaveText(declineNoteText);
    });
  } finally {
    await bobSession.close();
  }
});

test('Roym deal story: Alice agrees as listed and both Hubs see the agreement', async ({ browser, page }, testInfo) => {
  const env = readDemoEnv();
  const need = (key: string) => requireEnv(env, key, SCRIPT);
  const alice = { hub: need('ALICE_HUB_URL'), key: need('ALICE_KEY'), did: need('ALICE_DID'), addr: need('ALICE_ADDR') };
  const bob = {
    hub: need('BOB_HUB_URL'), key: need('BOB_KEY'), did: need('BOB_DID'), addr: need('BOB_ADDR'),
    directoryDid: need('BOB_DIRECTORY_DID'),
  };

  const runTag = Date.now().toString(36);
  const listingTitle = `Instant hedge trim (${runTag})`;
  let listingId = '';

  const bobSession = await newSession(browser, testInfo);
  const bobPage = await bobSession.newPage();

  try {
    await step(page, "Alice's screen: sign in", async () => {
      await page.goto(alice.hub);
      await page.waitForLoadState('networkidle');
      await loginWithDelegatedKey(page, alice.key);
    });

    await step(bobPage, "Bob's screen: sign in", async () => {
      await bobPage.goto(bob.hub);
      await bobPage.waitForLoadState('networkidle');
      await loginWithDelegatedKey(bobPage, bob.key);
    });

    await step(bobPage, "Bob's screen: publish an instant offering with its own terms", async () => {
      await ensureOwnSynOrg(bobPage, bob.directoryDid, 'language-lessons', 'en');
      listingId = await publishListing(
        bobPage, listingTitle, 'language-lessons', bob.directoryDid, 'en', undefined, bob.addr,
        {
          acceptance: 'instant',
          notLimited: true,
          cap: '5',
          quoteHours: '1',
          paymentTiming: 'after-work',
          cancellation: 'Free until 24 hours before',
          refund: 'Full refund',
          dispute: 'Write to us first',
        },
      );
    });

    await step(page, "Alice's screen: find the offering and open the agree-as-listed panel", async () => {
      const hit = await findListingHit(page, bob.directoryDid, 'language-lessons', listingId, listingTitle, 'en');
      await expect(hit.locator('.hit-agree-as-listed')).toBeVisible();
      await hit.locator('.hit-agree-as-listed').click();
      const panel = page.locator('.hit-as-listed');
      await expect(panel.locator('.as-listed-price')).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(panel).toContainText('Free until 24 hours before');
      await expect(panel).toContainText('Full refund');
      await expect(panel).toContainText('Write to us first');
      await expect(panel.locator('.as-listed-mt-warning')).toHaveCount(0);
    });

    await step(page, "Alice's screen: agree and send the listing's own terms", async () => {
      await page.locator('.hit-as-listed button.as-listed-send').click();
      await expect(page.locator('.messages-screen')).toBeVisible({ timeout: DELIVERY_WAIT_MS });
    });

    await step(page, "Alice's screen: the deal forms with no other action", async () => {
      await expect(runReceiptCard(page, runTag).first()).toBeVisible({ timeout: DELIVERY_WAIT_MS });
    });

    await step(bobPage, "Bob's screen: open the conversation and see the same agreement", async () => {
      await bobPage.bringToFront();
      await openMessages(bobPage);
      await openConversation(bobPage, alice.addr);
      await expect(runReceiptCard(bobPage, runTag).first()).toBeVisible({ timeout: DELIVERY_WAIT_MS });
    });

    await step(page, "Both Hubs hold the same two promised commitments", async () => {
      const aliceConversation = await conversationWith(page, bob.addr);
      const bobConversation = await conversationWith(bobPage, alice.addr);
      await expect
        .poll(async () => (await fetchAgreement(page, aliceConversation, runTag))?.commitments.length, {
          timeout: DELIVERY_WAIT_MS,
        })
        .toBe(2);
      await expect
        .poll(async () => (await fetchAgreement(bobPage, bobConversation, runTag))?.commitments.length, {
          timeout: DELIVERY_WAIT_MS,
        })
        .toBe(2);
    });
  } finally {
    await bobSession.close();
  }
});
