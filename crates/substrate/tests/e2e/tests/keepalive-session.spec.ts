import { test, expect } from '@playwright/test';

test.describe('Gateway Keep-Alive Session Routing (P1 Regression)', () => {
  test('browser on deployed service host reaches auth service without keep-alive stream collision', async ({ page }) => {
    const appAlias = process.env.APP_ALIAS;
    expect(appAlias).toBeDefined();

    const gatewayPort = 7660;
    const appUrl = appAlias.includes(':')
      ? `http://${appAlias}`
      : appAlias.endsWith('.localhost')
      ? `http://${appAlias}:${gatewayPort}`
      : `http://${appAlias}.localhost:${gatewayPort}`;
    const authUrl = `http://auth.localhost:${gatewayPort}`;

    // 1. Initial page navigation establishes an HTTP/1.1 keep-alive TCP connection
    // to the deployed application service via the client gateway.
    const response = await page.goto(`${appUrl}/`);
    expect(response?.status()).toBe(200);

    page.on('console', msg => console.log('BROWSER:', msg.text()));
    page.on('pageerror', err => console.log('PAGE ERROR:', err.message));

    // 2. From the loaded app page, fetch auth service challenge.
    // Under P1, reusing the existing connection to the previous upstream service would fail.
    // With dedicated auth hostname routing, the browser reaches the local auth service cleanly.
    const challenge = await page.evaluate(async (url) => {
      try {
        console.log('Fetching challenge from:', `${url}/_syneroym/session/challenge`);
        const res = await fetch(`${url}/_syneroym/session/challenge`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
        });
        return {
          status: res.status,
          data: await res.json(),
        };
      } catch (err: any) {
        console.error('Fetch error:', err.name, err.message);
        return { status: 0, error: String(err) };
      }
    }, authUrl);

    expect(challenge.status).toBe(200);
    expect(challenge.data.nonce).toBeDefined();
    expect(challenge.data.node_did).toBeDefined();

    // 3. Test bare /challenge route on the auth.localhost hostname.
    const bareChallenge = await page.evaluate(async (url) => {
      const res = await fetch(`${url}/challenge`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
      });
      return {
        status: res.status,
        data: await res.json(),
      };
    }, authUrl);

    expect(bareChallenge.status).toBe(200);
    expect(bareChallenge.data.nonce).toBeDefined();

    // 4. Verify unauthenticated GET /_syneroym/session/whoami returns 401 with expected error.
    const whoamiUnauth = await page.evaluate(async (url) => {
      const res = await fetch(`${url}/_syneroym/session/whoami`, {
        method: 'GET',
      });
      return {
        status: res.status,
        data: await res.json(),
      };
    }, authUrl);

    expect(whoamiUnauth.status).toBe(401);
    expect(whoamiUnauth.data.error).toBe('no session token provided');

    // 5. Verify GET /_syneroym/session/methods returns supported methods.
    const methods = await page.evaluate(async (url) => {
      const res = await fetch(`${url}/_syneroym/session/methods`, {
        method: 'GET',
      });
      return {
        status: res.status,
        data: await res.json(),
      };
    }, authUrl);

    expect(methods.status).toBe(200);
    expect(methods.data.methods).toContain('delegated-key');
  });
});
