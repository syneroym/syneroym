import { test, expect, Browser, Page } from '@playwright/test';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const APP_URL = (forceTunnel: boolean) =>
  `http://${process.env.WASM_APP_ALIAS}:7662/?force_tunnel=${forceTunnel}`;

// Opens a second, independent browser session on the same app and parks it on
// the comments page -- exit criteria 7 and 8 both say "a different browser
// session", which one page cannot demonstrate.
async function openSecondSession(browser: Browser, forceTunnel: boolean) {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(APP_URL(forceTunnel));
  // Same settle the primary session's beforeEach does (webrtc.spec.ts:23):
  // the service worker has to take control before the first proxied fetch.
  await page.waitForLoadState('networkidle');
  await expect(page.locator('h1')).toContainText('Hello world from demo1-instance0', { timeout: 30000 });
  await page.click('text=Comments etc.');
  await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
  return { context, page };
}

async function postComment(page: Page, text: string) {
  await page.fill('textarea[placeholder="Write a comment..."]', text);
  await page.click('button:has-text("Submit")');
  await expect(page.locator('text=Comment saved!')).toBeVisible({ timeout: 35000 });
}

[false, true].forEach(forceTunnel => {
  test.describe(`WASM SynApp E2E (forceTunnel=${forceTunnel})`, () => {
    // A two-session case pays for a second full WebRTC bootstrap
    // inside one test's budget, which the 30 s default cannot hold. Scoped
    // here rather than in playwright.config.ts so the native suite keeps its
    // proven, tighter budget.
    test.describe.configure({ timeout: 90_000 });

    test.beforeEach(async ({ page }) => {
      page.on('console', msg => console.log('BROWSER:', msg.text()));
      const wasmAppDid = process.env.WASM_APP_DID;
      expect(wasmAppDid).toBeDefined();

      const wasmAppAlias = process.env.WASM_APP_ALIAS;
      expect(wasmAppAlias).toBeDefined();

      const url = APP_URL(forceTunnel);
      console.log('Navigating to bootstrap URL:', url);

      await page.goto(url);
      await page.waitForLoadState('networkidle');
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
      await expect(page.locator('ul').first()).toContainText(commentText, { timeout: 35000 });

      // The UI half: App.tsx renders this on any non-ok response.
      await page.fill('textarea[placeholder="Write a comment..."]', '');
      await page.click('button:has-text("Submit")');
      await expect(page.locator('text=Error saving comment.')).toBeVisible({ timeout: 35000 });

      // The wire half: the guest's own status *and* message, which the
      // rendered string alone does not show.
      const rejected = await page.evaluate(async () => {
        const res = await fetch('/api/comments', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ text: '   ' }),
        });
        return { status: res.status, body: await res.text() };
      });
      expect(rejected.status).toBe(422);
      expect(rejected.body).toContain('text is required');
    });

    test('WebSocket Echo and Broadcast', async ({ page, browser }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      await expect(page.getByTestId('ws-status')).toHaveText('Connected', { timeout: 35000 });

      // Echo (unicast, guest -> this connection only): the client sends one
      // frame on open and the guest answers {"recdMsg":"Received: ..."}.
      await expect(page.getByTestId('ws-recd-msg'))
        .toHaveText('Received: Hi from client', { timeout: 20000 });

      // Broadcast from a different session.
      const before = await page.getByTestId('ws-last-updated').innerText();
      const other = await openSecondSession(browser, forceTunnel);
      try {
        await postComment(other.page, `WS broadcast ${Date.now()}`);
        await expect(page.getByTestId('ws-last-updated')).not.toHaveText(before, { timeout: 35000 });
      } finally {
        await other.context.close();
      }
    });

    test('File Upload and Download', async ({ page }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });

      // 256 KiB: several data-channel messages on the way up and several
      // `read-chunk` hops on the way down (exit criterion 6).
      const fileName = `wasm-upload-${Date.now()}.bin`;
      const filePath = path.join(os.tmpdir(), fileName);
      const bytes = Buffer.alloc(256 * 1024);
      for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251;
      fs.writeFileSync(filePath, bytes);
      try {
        await page.locator('input[type="file"]').setInputFiles(filePath);
        await page.click('button:has-text("Upload")');
        await expect(page.locator('text=Upload successful!')).toBeVisible({ timeout: 35000 });
        await expect(page.locator('ul').last()).toContainText(fileName, { timeout: 35000 });

        const result = await page.evaluate(async (name) => {
          const res = await fetch(`/api/files/${encodeURIComponent(name)}`);
          const buf = new Uint8Array(await res.arrayBuffer());
          let hash = 2166136261;
          for (const b of buf) {
            hash = Math.imul(hash ^ b, 16777619) >>> 0;
          }
          return { status: res.status, length: buf.length, hash };
        }, fileName);

        let expectedHash = 2166136261;
        for (const b of bytes) {
          expectedHash = Math.imul(expectedHash ^ b, 16777619) >>> 0;
        }
        expect(result.status).toBe(200);
        expect(result.length).toBe(bytes.length);
        expect(result.hash).toBe(expectedHash);
      } finally {
        if (fs.existsSync(filePath)) fs.unlinkSync(filePath);
      }
    });

    test('SSE receives an update published by another session', async ({ page, browser }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      await expect(page.getByTestId('sse-status')).toHaveText('Connected', { timeout: 15000 });
      await expect(page.getByTestId('sse-last-updated')).toHaveText('Never', { timeout: 15000 });

      const other = await openSecondSession(browser, forceTunnel);
      try {
        await postComment(other.page, `SSE update ${Date.now()}`);
        await expect(page.getByTestId('sse-last-updated')).not.toHaveText('Never', { timeout: 35000 });
      } finally {
        await other.context.close();
      }
    });
  });
});
