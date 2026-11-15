import { expect, test } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import {
  addContact, loginWithDelegatedKey, openConversation, openMessages, saveProfile, sendMessage,
  threadMessage,
} from '../hub-helpers';
import { DELIVERY_WAIT_MS, requireEnv, runScript, step, newSession } from './steps';

const SCRIPT = '20-roym-setup.sh';
// In attach mode the story can run again on the same nodes. A new tag in each message keeps the
// old messages from matching.
const RUN_TAG = Date.now().toString(36);
const ALICE_TEXT = `Hello Bob, this is Alice (${RUN_TAG})`;
const BOB_TEXT = `Hello Alice, this is Bob (${RUN_TAG})`;

test.describe.configure({ mode: 'serial' });

test('Roym chat story: deploy Roym on both nodes, in the terminal', async () => {
  runScript(SCRIPT);
});

test('Roym chat story: Alice and Bob talk, each on their own node', async ({ browser, page }, testInfo) => {
  const env = readDemoEnv();
  const need = (key: string) => requireEnv(env, key, SCRIPT);
  const alice = { hub: need('ALICE_HUB_URL'), key: need('ALICE_KEY'), did: need('ALICE_DID'), addr: need('ALICE_ADDR') };
  const bob = { hub: need('BOB_HUB_URL'), key: need('BOB_KEY'), did: need('BOB_DID'), addr: need('BOB_ADDR') };

  const bobSession = await newSession(browser, testInfo);
  const bobPage = await bobSession.newPage();

  try {
    // Two pages on the two Hubs. Each Hub is on its own node.
    await step(page, "Alice's screen: sign in and set up her profile", async () => {
      await page.goto(alice.hub);
      await page.waitForLoadState('networkidle');
      await loginWithDelegatedKey(page, alice.key);
      await saveProfile(page, 'Alice', alice.addr);
    });

    await step(page, "Alice's screen: add Bob as a contact", async () => {
      await addContact(page, bob.did, bob.addr);
    });

    await step(bobPage, "Bob's screen: sign in and set up his profile", async () => {
      await bobPage.goto(bob.hub);
      await bobPage.waitForLoadState('networkidle');
      await loginWithDelegatedKey(bobPage, bob.key);
      await saveProfile(bobPage, 'Bob', bob.addr);
    });

    await step(page, "Alice's screen: send Bob a message", async () => {
      await openMessages(page);
      await openConversation(page, bob.addr);
      await sendMessage(page, ALICE_TEXT);
      await expect(threadMessage(page, ALICE_TEXT).locator('.message-state')).toBeVisible();
    });

    await step(bobPage, "Bob's screen: open the conversation and read it", async () => {
      // Bob opens Messages once. Alice's message arrives and appears by itself.
      await openMessages(bobPage);
      await expect(threadMessage(bobPage, ALICE_TEXT)).toBeVisible({ timeout: DELIVERY_WAIT_MS });
    });

    await step(bobPage, "Bob's screen: reply to Alice", async () => {
      await sendMessage(bobPage, BOB_TEXT);
      await expect(threadMessage(bobPage, BOB_TEXT)).toBeVisible();
    });

    await step(page, "Alice's screen: her message is delivered, and Bob's reply is there", async () => {
      await page.bringToFront();
      // Alice kept her conversation open. Bob's reply and the delivery state appear by themselves.
      await expect(threadMessage(page, BOB_TEXT)).toBeVisible({ timeout: DELIVERY_WAIT_MS });
      await expect(threadMessage(page, ALICE_TEXT).locator('.message-state')).toHaveText('delivered', {
        timeout: DELIVERY_WAIT_MS,
      });
    });
  } finally {
    await bobSession.close();
  }
});
