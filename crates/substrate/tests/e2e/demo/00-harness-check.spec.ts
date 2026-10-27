import { expect, test } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';

test('both nodes answer', async () => {
  const env = readDemoEnv();
  for (const url of [env.REGISTRY + '/', `http://127.0.0.1:${env.S1_GW}/`, `http://127.0.0.1:${env.S2_GW}/`]) {
    const res = await fetch(url);
    console.log(url, res.status);
    expect(res.status).toBeLessThan(500);
  }
});
