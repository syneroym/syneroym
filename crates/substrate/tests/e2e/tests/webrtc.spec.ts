import { test, expect } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

[false, true].forEach(forceTunnel => {
  test.describe(`WebRTC Substrate E2E (forceTunnel=${forceTunnel})`, () => {
    test.beforeEach(async ({ page }) => {
      page.on('console', msg => console.log('BROWSER:', msg.text()));
      const appDid = process.env.APP_DID;
      expect(appDid).toBeDefined();

      const appAlias = process.env.APP_ALIAS;
      expect(appAlias).toBeDefined();

      // The bootstrap page is served by the coordinator on port 7662.
      // We access it via the alias hostname to ensure the coordinator can resolve it.
      const url = `http://${appAlias}:7662/?force_tunnel=${forceTunnel}`;
      console.log('Navigating to bootstrap URL:', url);

      await page.goto(url);

      // Give it some time to establish WebRTC and load the proxied content
      await page.waitForLoadState('networkidle');

      // The mock app's index page returns "<h1>Hello world from demo1-instance0</h1>..."
      // Wait for that content to appear, meaning the Service Worker is active and WebRTC proxying is working
      await expect(page.locator('h1')).toContainText('Hello world from demo1-instance0', { timeout: 30000 });
    });

    test('GET / and navigate to comments', async ({ page }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
    });

    test('POST /api/comments and verify recent comments', async ({ page }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      const commentText = `Test comment from Playwright ${Date.now()}`;
      
      await page.fill('textarea[placeholder="Write a comment..."]', commentText);
      await page.click('button:has-text("Submit")');
      
      await expect(page.locator('text=Comment saved!')).toBeVisible({ timeout: 35000 });
      
      // Verify it appears in the list
      await expect(page.locator('ul').first()).toContainText(commentText, { timeout: 35000 });
    });

    test('WebSocket Echo and Broadcast', async ({ page }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      
      // Wait for WebSocket to connect before submitting, to avoid missing the broadcast
      await expect(page.locator('text=Live Updates: Connected')).toBeVisible({ timeout: 15000 });

      // The miniapp-demo1-web has a WebSocket echo feature.
      // When a comment is saved, it broadcasts a timestamp.
      
      const commentText = `Broadcast Test ${Date.now()}`;
      
      // We expect a broadcast message in the "Live Updates" component
      const lastUpdatedLocator = page.locator('div:has-text("Live Updates:") span').first();
      const lastUpdatedBefore = await lastUpdatedLocator.innerText();
      
      await page.fill('textarea[placeholder="Write a comment..."]', commentText);
      await page.click('button:has-text("Submit")');
      
      // Wait for the broadcast message to update the UI
      await expect(lastUpdatedLocator).not.toHaveText(lastUpdatedBefore, { timeout: 35000 });
    });

    test('File Upload and Download', async ({ page }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      
      const fileName = `test-file-${Date.now()}.txt`;
      const filePath = path.join(__dirname, fileName);
      fs.writeFileSync(filePath, 'Hello from PlayRTC!');

      try {
        // Upload
        const fileInput = page.locator('input[type="file"]');
        await fileInput.setInputFiles(filePath);
        await page.click('button:has-text("Upload")');

        await expect(page.locator('text=Upload successful!')).toBeVisible({ timeout: 35000 });

        // Verify in list and Download content
        await expect(page.locator('ul').last()).toContainText(fileName, { timeout: 35000 });
        
        const content = await page.evaluate(async (name) => {
          const res = await fetch(`/api/files/${name}`);
          return await res.text();
        }, fileName);
        
        expect(content).toBe('Hello from PlayRTC!');
      } finally {
        if (fs.existsSync(filePath)) fs.unlinkSync(filePath);
      }
    });
  });
});
