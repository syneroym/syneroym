import { test, expect } from '@playwright/test';

test.describe('WebRTC Substrate E2E', () => {
  test('should load proxied content via WebRTC Bootstrap Page', async ({ page }) => {
    const appDid = process.env.APP_DID;
    expect(appDid).toBeDefined();

    const appAlias = process.env.APP_ALIAS;
    expect(appAlias).toBeDefined();

    // The bootstrap page is served by the coordinator on port 7662.
    // We access it via the alias hostname to ensure the coordinator can resolve it.
    const url = `http://${appAlias}:7662/`;
    console.log('Navigating to bootstrap URL:', url);

    await page.goto(url);

    // Give it some time to establish WebRTC and load the proxied content
    await page.waitForLoadState('networkidle');

    // The mock app's index page returns "<h1>Hello world from demo1-instance0</h1>..."
    // Wait for that content to appear, meaning the Service Worker is active and WebRTC proxying is working
    await expect(page.locator('h1')).toContainText('Hello world from demo1-instance0', { timeout: 15000 });
  });
});
