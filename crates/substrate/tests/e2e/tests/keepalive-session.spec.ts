import { test, expect } from '@playwright/test';

test.describe('Gateway Keep-Alive Session Routing', () => {
  test('retains proper host-based dispatch when Host changes across requests', async ({ request }) => {
    const gatewayPort = 7660;

    // 1. Request to auth host
    const res1 = await request.post(`http://127.0.0.1:${gatewayPort}/_syneroym/session/challenge`, {
      headers: {
        'Host': 'auth.localhost',
        'Connection': 'keep-alive',
      },
    });

    expect(res1.status()).toBe(200);
    const data1 = await res1.json();
    expect(data1.nonce).toBeDefined();

    // 2. Query methods on auth host over keep-alive connection
    const res2 = await request.get(`http://127.0.0.1:${gatewayPort}/_syneroym/session/methods`, {
      headers: {
        'Host': 'auth.localhost',
        'Connection': 'keep-alive',
      },
    });
    expect(res2.status()).toBe(200);
    const data2 = await res2.json();
    expect(data2.methods).toContain('delegated-key');
  });
});
