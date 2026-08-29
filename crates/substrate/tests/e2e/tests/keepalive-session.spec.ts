import { test, expect } from '@playwright/test';

test.describe('Gateway Keep-Alive Session Routing (P1 Regression)', () => {
  test('browser keep-alive connection answers session requests after page load', async ({ page }) => {
    const gatewayUrl = 'http://127.0.0.1:7660';

    // 1. Initial navigation loads session methods over HTTP/1.1 keep-alive
    const response = await page.goto(`${gatewayUrl}/_syneroym/session/methods`);
    expect(response?.status()).toBe(200);
    const body = await response?.json();
    expect(body.methods).toContain('delegated-key');

    // 2. While the browser's keep-alive TCP socket is maintained,
    // execute in-page fetch for POST /_syneroym/session/challenge
    const challenge = await page.evaluate(async (url) => {
      const res = await fetch(`${url}/_syneroym/session/challenge`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
      });
      return {
        status: res.status,
        data: await res.json(),
      };
    }, gatewayUrl);

    expect(challenge.status).toBe(200);
    expect(challenge.data.nonce).toBeDefined();

    // 3. Execute subsequent in-page fetch for GET /_syneroym/session/whoami
    const whoami = await page.evaluate(async (url) => {
      const res = await fetch(`${url}/_syneroym/session/whoami`, {
        method: 'GET',
      });
      return {
        status: res.status,
        data: await res.json(),
      };
    }, gatewayUrl);

    expect(whoami.status).toBe(200);
    expect(whoami.data.auth).toBe('unauthenticated');
  });
});
