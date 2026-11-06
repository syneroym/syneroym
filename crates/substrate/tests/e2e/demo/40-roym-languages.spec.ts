import { expect, test } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import { chooseHubLanguage, hubMessage, loginWithDelegatedKey } from '../hub-helpers';
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
});
