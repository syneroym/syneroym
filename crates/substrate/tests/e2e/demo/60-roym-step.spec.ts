import { expect, test, type Locator, type Page } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import {
  conversationWith, ensureOwnSynOrg, fetchAgreement, loginWithDelegatedKey,
  openConversation, openMessages, publishListing, rpcCall, sendRequest, threadMessage,
} from '../hub-helpers';
import { DELIVERY_WAIT_MS, newSession, requireEnv, runScript, step } from './steps';

const SCRIPT = '20-roym-setup.sh';

test.describe.configure({ mode: 'serial' });

/** The receipt cards of this run in the open thread. The scope on a card holds the run tag. */
function runReceiptCard(page: Page, runTag: string): Locator {
  return page.locator('.thread-messages .card-agreement-receipt', { hasText: runTag });
}

/** The step cards in the open thread. */
function stepCard(page: Page): Locator {
  return page.locator('.thread-messages .card-step');
}

/** Tomorrow 10:00 to 11:00 in the browser's time zone, as `datetime-local` values. */
async function tomorrowSlot(page: Page): Promise<{ start: string; end: string }> {
  return page.evaluate(() => {
    const pad = (n: number) => String(n).padStart(2, '0');
    const local = (d: Date) =>
      `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
    const start = new Date();
    start.setDate(start.getDate() + 1);
    start.setHours(10, 0, 0, 0);
    return { start: local(start), end: local(new Date(start.getTime() + 3_600_000)) };
  });
}

test('Roym step story: the giver marks work started and both Hubs show the card', async ({ browser, page }, testInfo) => {
  const env = readDemoEnv();
  const need = (key: string) => requireEnv(env, key, SCRIPT);
  const alice = {
    hub: need('ALICE_HUB_URL'), key: need('ALICE_KEY'), addr: need('ALICE_ADDR'),
    directoryDid: need('ALICE_DIRECTORY_DID'),
  };
  const bob = { hub: need('BOB_HUB_URL'), key: need('BOB_KEY'), did: need('BOB_DID'), addr: need('BOB_ADDR') };

  const runTag = Date.now().toString(36);
  const listingTitle = `Garden clearance (${runTag})`;
  const enquiryText = `Clear my back garden (${runTag})`;

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

    await step(page, "Alice's screen: publish a listing and add one time slot for tomorrow", async () => {
      await ensureOwnSynOrg(page, alice.directoryDid, 'language-lessons', 'en');
      const listingId = await publishListing(
        page, listingTitle, 'language-lessons', alice.directoryDid, 'en', undefined, alice.addr,
      );
      const slot = await tomorrowSlot(page);
      const row = page.locator('.listing-row', { hasText: listingTitle }).first();
      await row.locator('input[type="datetime-local"]').nth(0).fill(slot.start);
      await row.locator('input[type="datetime-local"]').nth(1).fill(slot.end);
      await row.locator('button.add-slot').click();
      await expect(row.locator('.slot-line')).toHaveCount(1, { timeout: 15_000 });

      // The slot id and its window come from the node, so the quote names the slot exactly.
      const slots = await rpcCall(page, 'availability.list', { listing_id: listingId });
      const added = slots.result?.slots?.[0];
      expect(added?.slot_id, 'the node lists the slot').toBeTruthy();
      process.env.DEMO_STEP_LISTING_ID = listingId;
      process.env.DEMO_STEP_SLOT_ID = added.slot_id;
      process.env.DEMO_STEP_SLOT_START = String(added.start_secs);
      process.env.DEMO_STEP_SLOT_END = String(added.end_secs);
    });

    await step(bobPage, "Bob's screen: send Alice an enquiry", async () => {
      await openMessages(bobPage);
      await openConversation(bobPage, alice.addr);
      await sendRequest(bobPage, enquiryText, 'language-lessons');
    });

    await step(page, "Alice's screen: the enquiry arrives", async () => {
      await openMessages(page);
      await openConversation(page, bob.addr);
      await expect(threadMessage(page, enquiryText)).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      const conversation = await conversationWith(page, bob.addr);
      await rpcCall(page, 'transaction.sync', { conversation });
      const thread = await rpcCall(page, 'transaction.thread', { conversation });
      const enquiry = (thread.result?.cards ?? []).find(
        (c: { card_type: string; issuer?: string; data?: unknown }) =>
          c.card_type === 'enquiry' && c.issuer === bob.did && JSON.stringify(c.data).includes(runTag),
      );
      expect(enquiry?.record_id, "Bob's enquiry is filed on Alice's node").toBeTruthy();
      process.env.DEMO_STEP_ENQUIRY_ID = enquiry.record_id;
      process.env.DEMO_STEP_TAG = runTag;
    });

    await step(page, "Terminal: Alice sends Bob a quote for the slot", async () => {
      runScript('60-roym-step-quote.sh');
    });

    await step(bobPage, "Bob's screen: the quote shows the slot's reference codes and Bob accepts", async () => {
      await bobPage.bringToFront();
      const quote = threadMessage(bobPage, `Garden clearance ${runTag}`);
      await expect(quote).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(quote.locator('.quote-details')).toContainText(process.env.DEMO_STEP_SLOT_ID!);
      await quote.locator('.accept-quote-button').click();
      await expect(bobPage.locator('.accept-mt-box')).toHaveCount(0);
      await expect(runReceiptCard(bobPage, runTag).first()).toBeVisible({ timeout: 15_000 });
    });

    await step(page, "Alice's screen: the agreement is formed on both nodes", async () => {
      await page.bringToFront();
      const aliceConversation = await conversationWith(page, bob.addr);
      let agreementId = '';
      await expect
        .poll(async () => {
          const agr = await fetchAgreement(page, aliceConversation, runTag);
          agreementId = agr?.quote_record_id ?? '';
          return agr?.commitments.map((c) => c.state);
        }, { timeout: DELIVERY_WAIT_MS })
        .toEqual(['promised', 'promised']);
      process.env.DEMO_STEP_AGREEMENT_ID = agreementId;

      const bobConversation = await conversationWith(bobPage, alice.addr);
      await expect
        .poll(async () => {
          const agr = await fetchAgreement(bobPage, bobConversation, runTag);
          return agr?.commitments.map((c) => c.state);
        }, { timeout: DELIVERY_WAIT_MS })
        .toEqual(['promised', 'promised']);
    });

    await step(page, "Terminal: Alice marks the work as started", async () => {
      runScript('60-roym-step-start.sh');
    });

    await step(page, "Alice's screen: the Work started card is in the chat", async () => {
      await page.bringToFront();
      await expect(stepCard(page).first()).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(stepCard(page).first()).toContainText('Work started');
    });

    await step(bobPage, "Bob's screen: the same card arrives", async () => {
      await bobPage.bringToFront();
      await expect(stepCard(bobPage).first()).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(stepCard(bobPage).first()).toContainText('Work started');
    });
  } finally {
    await bobSession.close();
  }
});
