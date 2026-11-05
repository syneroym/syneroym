import { chromium } from '@playwright/test';
import { test, expect } from '../fixtures';
import { loginWithDelegatedKey } from '../hub-helpers';
import { readE2EPorts } from '../ports';

// The Hub opened from a machine other than the node's own: a host name on
// another domain, not `*.localhost`. Chromium maps every `*.roym.test` name to
// the loopback address, so one machine can stand in for two.

test.describe('Roym Hub on another domain', () => {
  test('42. a Hub opened on another domain reaches its own auth origin and nothing else', async () => {
    const port = readE2EPorts().gatewayPort;
    expect(process.env.ROYM_WEB_ALIAS).toBeDefined();
    const hubLabel = process.env.ROYM_WEB_ALIAS!.split('.')[0];
    const hubOrigin = `http://${hubLabel}.roym.test:${port}`;
    const authOrigin = `http://auth.roym.test:${port}`;

    // `crypto.subtle` exists only in a secure context. A real remote Hub is
    // served over HTTPS; here the two plain-HTTP origins are treated as secure.
    // The full Chromium channel is needed: the default headless shell ignores
    // this flag.
    const browser = await chromium.launch({
      channel: 'chromium',
      args: [
        '--host-resolver-rules=MAP *.roym.test 127.0.0.1',
        `--unsafely-treat-insecure-origin-as-secure=${hubOrigin},${authOrigin}`,
      ],
    });
    try {
      const page = await (await browser.newContext()).newPage();
      await page.goto(hubOrigin);
      await page.waitForLoadState('networkidle');
      await expect(page.locator('h1')).toHaveText('Roym Hub');

      // 1. The Hub's own auth service answers: the CSP and the CORS rule allow it.
      const methodsStatus = await page.evaluate(
        async (origin) => (await fetch(`${origin}/_syneroym/session/methods`)).status,
        authOrigin,
      );
      expect(methodsStatus).toBe(200);

      // 2. Any other origin is blocked by the CSP. The event is the proof: a
      // rejected `fetch` alone also happens for lack of CORS headers, under
      // the wide static tag. Only the narrower second tag raises this event.
      const violation = await page.evaluate(
        (target) =>
          new Promise<{ directive: string; blocked: string }>((resolve) => {
            document.addEventListener('securitypolicyviolation', (e) =>
              resolve({ directive: e.effectiveDirective, blocked: e.blockedURI }),
            );
            fetch(target).catch(() => undefined);
          }),
        `http://other.roym.test:${port}/`,
      );
      expect(violation.directive).toBe('connect-src');
      expect(violation.blocked).toContain('other.roym.test');

      // 3. A person signs in with delegated-key on that origin.
      await page.reload();
      await loginWithDelegatedKey(page);
    } finally {
      await browser.close();
    }
  });
});
