import { expect, test } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import {
  addListingTranslation, addMessageLanguage, chooseHubLanguage, directoryAddedStatus, ensureOwnSynOrg,
  hubMessage, loginWithDelegatedKey, openConversation, openMessages, pickCategory, publishListing,
  sendMessage,
} from '../hub-helpers';
import { requireEnv, step } from './steps';

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
