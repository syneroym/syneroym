import { test, expect } from '../fixtures';
import * as fs from 'fs';
import * as path from 'path';
import { readE2EPorts } from '../ports';

// The channel functions the page ships, loaded from the template file.
// This checks the code the browser runs, not a copy of it.
function loadPageChannelFunctions() {
  const file = path.resolve(__dirname, '../../../../coordinator_webrtc/templates/peer-proxy.js');
  const src = fs.readFileSync(file, 'utf8');
  const body = src.slice(
    src.indexOf('const KDF_SALT'),
    src.indexOf('async function verifyAndDeriveSharedSecret'),
  );
  return new Function(
    'window',
    `${body}; return { deriveChannelKeys, handshakeSignedBytes, httpRefusalResponse, refusalReason };`,
  )({ crypto: globalThis.crypto });
}

// Pinned in `crates/router/src/e2e_channel/tests.rs` (`kdf_known_answer`).
const PINNED_C2S = 'd2f3bfc8d7382442a0d68fadfa725144e589164a57a1cd7de9f4fd37723bfc86';
const PINNED_S2C = 'edf347eb9a7ea22d897d206517cf4ff7055d9eab2759cbc4cdd6e2ff197790c9';

test('page channel functions match the router', async () => {
  const page = loadPageChannelFunctions();
  const subtle = globalThis.crypto.subtle;
  const clientPub = new Uint8Array(65).fill(2);
  clientPub[0] = 0x04;
  const serverPub = new Uint8Array(65).fill(3);
  serverPub[0] = 0x04;
  const keys = await page.deriveChannelKeys(new Uint8Array(32).fill(1).buffer, clientPub.buffer, serverPub);

  // The page's keys are not extractable, so each one is checked against
  // the pinned key by a round trip.
  const pinned = (hex: string, usage: KeyUsage) =>
    subtle.importKey('raw', Buffer.from(hex, 'hex'), 'AES-GCM', false, [usage]);
  const iv = new Uint8Array(12);
  const msg = new TextEncoder().encode('kdf parity');
  const c2s = await subtle.encrypt({ name: 'AES-GCM', iv }, keys.sendKey, msg);
  const c2sPlain = await subtle.decrypt({ name: 'AES-GCM', iv }, await pinned(PINNED_C2S, 'decrypt'), c2s);
  expect(Buffer.from(c2sPlain).toString()).toBe('kdf parity');
  const s2c = await subtle.encrypt({ name: 'AES-GCM', iv }, await pinned(PINNED_S2C, 'encrypt'), msg);
  const s2cPlain = await subtle.decrypt({ name: 'AES-GCM', iv }, keys.recvKey, s2c);
  expect(Buffer.from(s2cPlain).toString()).toBe('kdf parity');

  // Same layout as `e2e_channel::handshake_signed_bytes`.
  const signed = page.handshakeSignedBytes(serverPub, clientPub.buffer);
  const tag = Buffer.from('syneroym-e2e-handshake-v1');
  expect(Buffer.from(signed.slice(0, tag.length)).equals(tag)).toBe(true);
  expect(Buffer.from(signed.slice(tag.length)).equals(Buffer.concat([serverPub, clientPub]))).toBe(true);

  // A refusal is accepted only with an error status and a known reason,
  // and the page shows its own copy of it.
  expect(page.refusalReason('HTTP/1.1 200 OK\r\nx-syneroym-refusal: unauthenticated')).toBeNull();
  expect(page.refusalReason('HTTP/1.1 401 Unauthorized\r\ncontent-type: text/html')).toBeNull();
  expect(page.refusalReason('HTTP/1.1 401 Unauthorized\r\nX-Syneroym-Refusal: unauthenticated'))
    .toBe('unauthenticated');
  expect(page.httpRefusalResponse('forged')).toBeNull();
  const response = Buffer.from(page.httpRefusalResponse('not_hosted_here')).toString();
  expect(response.startsWith('HTTP/1.1 421 Misdirected Request\r\n')).toBe(true);
  expect(JSON.parse(response.split('\r\n\r\n')[1]).error.code).toBe(-32096);
});

[false, true].forEach(forceTunnel => {
  test.describe(`WebRTC Substrate E2E (forceTunnel=${forceTunnel})`, () => {
    test.beforeEach(async ({ page }) => {
      const appDid = process.env.APP_DID;
      expect(appDid).toBeDefined();

      const appAlias = process.env.APP_ALIAS;
      expect(appAlias).toBeDefined();

      const ports = readE2EPorts();
      // The bootstrap page is served by the coordinator on its dynamic bootstrap port.
      // We access it via the alias hostname to ensure the coordinator can resolve it.
      const url = `http://${appAlias}:${ports.webrtcBootstrapPort}/?force_tunnel=${forceTunnel}`;
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
