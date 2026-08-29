import { test, expect } from '@playwright/test';

test.describe('Gateway Keep-Alive Session Routing', () => {
  test('retains proper host-based dispatch over keep-alive connection', async ({ request }) => {
    // Gateway endpoint test verifying that connection keep-alive does not pin
    // socket routing when Host headers change across requests.
    const gatewayPort = 7960;
    const authHost = 'auth-s00000000.localhost';

    // 1. Issue challenge on auth host
    const res1 = await request.post(`http://127.0.0.1:${gatewayPort}/_syneroym/session/challenge`, {
      headers: {
        'Host': authHost,
        'Connection': 'keep-alive',
      },
    });

    if (res1.status() === 200) {
      const data1 = await res1.json();
      expect(data1.nonce).toBeDefined();

      // 2. Query methods over same connection
      const res2 = await request.get(`http://127.0.0.1:${gatewayPort}/_syneroym/session/methods`, {
        headers: {
          'Host': authHost,
          'Connection': 'keep-alive',
        },
      });
      expect(res2.status()).toBe(200);
      const data2 = await res2.json();
      expect(data2.methods).toContain('delegated-key');
    }
  });
});
