import { expect, test } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import {
  addContact, loginWithDelegatedKey, openConversation, openMessages, saveProfile, sendMessage,
  threadMessage,
} from '../hub-helpers';
import { requireEnv, runScript, step } from './steps';

const SCRIPT = '20-roym-setup.sh';
// A message crosses from one node to the other on the sender's outbox tick, which the demo
// nodes set to one second. The wait is far longer, for a slow machine.
const DELIVERY_WAIT_MS = 60_000;

test.describe.configure({ mode: 'serial' });

test('Roym chat story: deploy Roym on both nodes, in the terminal', async () => {
  runScript(SCRIPT);
});

test('Roym chat story: Alice and Bob talk, each on their own node', async ({ page }) => {
  const env = readDemoEnv();
  const need = (key: string) => requireEnv(env, key, SCRIPT);
  const alice = { hub: need('ALICE_HUB_URL'), key: need('ALICE_KEY'), did: need('ALICE_DID'), addr: need('ALICE_ADDR') };
  const bob = { hub: need('BOB_HUB_URL'), key: need('BOB_KEY'), did: need('BOB_DID'), addr: need('BOB_ADDR') };

  // One tab goes between the two Hubs. Each Hub is its own origin, so each keeps its own session.
  await step(page, "Alice's screen: sign in and set up her profile", async () => {
    await page.goto(alice.hub);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page, alice.key);
    await saveProfile(page, 'Alice', alice.addr);
  });

  await step(page, "Alice's screen: add Bob as a contact", async () => {
    await addContact(page, bob.did, bob.addr);
  });

  await step(page, "Bob's screen: sign in and set up his profile", async () => {
    await page.goto(bob.hub);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page, bob.key);
    await saveProfile(page, 'Bob', bob.addr);
  });

  await step(page, "Alice's screen: send Bob a message", async () => {
    await page.goto(alice.hub);
    await page.waitForLoadState('networkidle');
    await openMessages(page);
    await openConversation(page, bob.addr);
    await sendMessage(page, 'Hello Bob, this is Alice');
    await expect(threadMessage(page, 'Hello Bob').locator('.message-state')).toBeVisible();
  });

  await step(page, "Bob's screen: open the conversation and read it", async () => {
    await page.goto(bob.hub);
    await page.waitForLoadState('networkidle');
    // The Hub does not update live. Open the screen again until the message has arrived.
    await expect(async () => {
      await openMessages(page);
      await expect(threadMessage(page, 'Hello Bob')).toBeVisible({ timeout: 2_000 });
    }).toPass({ timeout: DELIVERY_WAIT_MS });
  });

  await step(page, "Bob's screen: reply to Alice", async () => {
    await sendMessage(page, 'Hello Alice, this is Bob');
    await expect(threadMessage(page, 'Hello Alice')).toBeVisible();
  });

  await step(page, "Alice's screen: her message is delivered, and Bob's reply is there", async () => {
    await page.goto(alice.hub);
    await page.waitForLoadState('networkidle');
    await expect(async () => {
      await openMessages(page);
      // Alice has one conversation, and the Hub opens it first.
      await expect(threadMessage(page, 'Hello Alice')).toBeVisible({ timeout: 2_000 });
    }).toPass({ timeout: DELIVERY_WAIT_MS });
    await expect(threadMessage(page, 'Hello Bob').locator('.message-state')).toHaveText('delivered', {
      timeout: DELIVERY_WAIT_MS,
    });
  });
});
