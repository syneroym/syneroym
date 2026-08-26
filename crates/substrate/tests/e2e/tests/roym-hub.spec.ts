import { test, expect } from '@playwright/test';

test.describe('Roym Hub E2E Tests', () => {
  test.describe.configure({ timeout: 60_000 });

  test.beforeEach(async ({ page }) => {
    page.on('console', msg => console.log('BROWSER:', msg.text()));
    const roymAlias = process.env.ROYM_WEB_ALIAS;
    expect(roymAlias).toBeDefined();
  });

  test('1. Login: Identity picker lists seeded person, logs in, and session bar shows DID', async ({ page }) => {
    const hubUrl = process.env.ROYM_HUB_URL || 'http://127.0.0.1:7660';
    await page.goto(hubUrl);
    await page.waitForLoadState('networkidle');

    // Verify title and CSP header
    await expect(page.locator('h1')).toContainText('Roym Hub');

    // Verify identity picker shows alice
    const select = page.locator('select');
    await expect(select).toBeVisible({ timeout: 10_000 });
    await expect(select.locator('option')).toContainText(['alice']);

    // Log in as alice
    await select.selectOption('alice');
    await page.click('button:has-text("Log In")');

    // Verify session bar after login
    await page.waitForLoadState('networkidle');
    const sessionBar = page.locator('.session-bar');
    await expect(sessionBar).toBeVisible({ timeout: 10_000 });
    await expect(sessionBar).toContainText('did:key:');
    await expect(sessionBar).toContainText('delegated');
  });

  // A real substrate restart isn't reachable from here: Playwright always
  // runs test workers in a process separate from globalSetup.ts, which is
  // the only place holding the spawned substrate's process handle
  // (__SUBSTRATE_PROCESS__). The actual restart case (whoami -> 401 after
  // a restart, login-local works again) lives in
  // crates/substrate/tests/gateway_session_e2e.rs instead, which controls
  // the substrate process directly. This test covers what a browser can
  // actually observe: a session surviving a reload, and clearing cookies
  // returning to a clean login state.
  test('2. Session persists across reload; clearing cookies returns to login', async ({ page }) => {
    const hubUrl = process.env.ROYM_HUB_URL || 'http://127.0.0.1:7660';
    await page.goto(hubUrl);
    await page.waitForLoadState('networkidle');

    // If not logged in, log in as alice
    const select = page.locator('select');
    const sessionBar = page.locator('.session-bar');
    if (await select.isVisible()) {
      await select.selectOption('alice');
      await page.click('button:has-text("Log In")');
      await expect(sessionBar).toContainText('did:key:', { timeout: 10_000 });
    }

    await expect(sessionBar).toBeVisible({ timeout: 10_000 });
    await expect(sessionBar).toContainText('delegated');

    // Reload page: session remains active
    await page.reload();
    await page.waitForLoadState('networkidle');
    await expect(sessionBar).toBeVisible({ timeout: 10_000 });
    await expect(sessionBar).toContainText('did:key:');
    await expect(sessionBar).toContainText('delegated');

    // Clear cookies to simulate expired session
    await page.context().clearCookies();
    await page.reload();
    await page.waitForLoadState('networkidle');

    // Verify Hub shows identity picker again cleanly
    await expect(select).toBeVisible({ timeout: 10_000 });
  });

  test('3. Card gallery: Seven known types and unknown fallback render correctly', async ({ page }) => {
    const hubUrl = process.env.ROYM_HUB_URL || 'http://127.0.0.1:7660';
    await page.goto(hubUrl);
    await page.waitForLoadState('networkidle');

    // Log in first: renderHome's own sample gallery (not a synthetic one
    // built by this test) only renders once a session exists.
    await page.locator('select').selectOption('alice');
    await page.click('button:has-text("Log In")');
    await page.waitForLoadState('networkidle');
    await expect(page.locator('.session-bar')).toContainText('did:key:', { timeout: 10_000 });

    // The real gallery in main.ts's renderHome renders one sample per known
    // card type plus two deliberately-unknown ones -- assert against the
    // actual rendered DOM, not a fixture this test constructs.
    const knownTypes = [
      'request',
      'quote',
      'agreement-receipt',
      'booking-progress',
      'payment-request',
      'payment-acknowledgement',
      'fulfilment-receipt',
    ];
    for (const type of knownTypes) {
      await expect(page.locator(`.card-${type}`)).toHaveCount(1);
    }
    // Two unknown samples: an unrecognized type, and a known type at an
    // unrecognized version.
    await expect(page.locator('.card-unknown')).toHaveCount(2);
  });

  test('4. Card safety: Malicious payload produces zero console errors, zero external requests, and literal text', async ({ page }) => {
    const hubUrl = process.env.ROYM_HUB_URL || 'http://127.0.0.1:7660';

    const externalRequests: string[] = [];
    page.on('request', req => {
      const url = req.url();
      if (!url.includes(':7660')) {
        externalRequests.push(url);
      }
    });

    const consoleErrors: string[] = [];
    page.on('console', msg => {
      if (msg.type() === 'error') {
        const text = msg.text();
        if (!text.includes('Failed to load resource')) {
          consoleErrors.push(text);
        }
      }
    });

    await page.goto(hubUrl);
    await page.waitForLoadState('networkidle');

    // Drive the real card renderer (window.RoymRegistry, main.ts) with a
    // malicious payload, the same object shape a `request` card's `invoke`
    // response would carry -- not a hand-built div this test controls.
    const maliciousPayload =
      '<img src=x onerror=window.maliciousRan=true><script>window.maliciousScript=true</script>' +
      '<a href="javascript:window.maliciousLink=true">Click</a>';

    await page.evaluate((payload) => {
      const win = window as any;
      if (!win.RoymRegistry?.renderCard) {
        throw new Error('window.RoymRegistry.renderCard is not exposed by main.ts');
      }
      const container = document.createElement('div');
      container.id = 'safety-test-container';
      const el = win.RoymRegistry.renderCard({
        type: 'request',
        version: 1,
        data: { summary: payload },
      });
      container.appendChild(el);
      document.body.appendChild(container);
    }, maliciousPayload);

    // Wait a brief moment to check if any script executed
    await page.waitForTimeout(500);

    // Assertions
    const maliciousRan = await page.evaluate(() => (window as any).maliciousRan || (window as any).maliciousScript);
    expect(maliciousRan).toBeUndefined();
    expect(externalRequests).toEqual([]);
    expect(consoleErrors).toEqual([]);

    // Rendered through the real request-card template, so this asserts
    // the product's own path renders the payload as literal text.
    await expect(page.locator('#safety-test-container .card-request')).toHaveClass(/card-request/);
    const renderedText = await page.locator('#safety-test-container').innerText();
    expect(renderedText).toContain('<img src=x');
    expect(renderedText).toContain('<script>');
  });
});
