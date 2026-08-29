import { test, expect, type Page } from '@playwright/test';

const HUB_URL = process.env.ROYM_HUB_URL || 'http://127.0.0.1:7660';
const SESSION_KEY_FILE = process.env.ROYM_SESSION_KEY_FILE;

// Drive the real delegated-key login: hand the Hub the session-key.json that
// `global-setup`'s `roymctl session delegate` produced, exactly as a person
// would after running the command themselves.
async function loginWithDelegatedKey(page: Page) {
  await expect(page.locator('.login-picker h2')).toHaveText('Sign in');
  expect(SESSION_KEY_FILE, 'ROYM_SESSION_KEY_FILE must be set by global-setup').toBeTruthy();
  await page.locator('input[type="file"]').setInputFiles(SESSION_KEY_FILE!);
  await expect(page.locator('.session-bar')).toContainText('did:key:', { timeout: 15_000 });
  await expect(page.locator('.session-bar')).toContainText('delegated');
}

test.describe('Roym Hub', () => {
  test.beforeEach(async ({ page }) => {
    page.on('console', msg => console.log('BROWSER:', msg.text()));
    expect(process.env.ROYM_WEB_ALIAS).toBeDefined();
  });

  test('1. delegated-key login: import a session key, session bar shows the person DID', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await expect(page.locator('h1')).toHaveText('Roym Hub');
    await loginWithDelegatedKey(page);
  });

  test('2. session survives a reload; clearing browser state returns to login', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    // A plain reload keeps the session (the cookie rides along).
    await page.reload();
    await page.waitForLoadState('networkidle');
    await expect(page.locator('.session-bar')).toContainText('did:key:', { timeout: 15_000 });

    // Clearing cookies and IndexedDB drops both the session and the stored
    // key, so the Hub is back to a clean "Sign in" screen -- an ordinary
    // state, not an error.
    await page.context().clearCookies();
    await page.evaluate(() => indexedDB.deleteDatabase('roym-hub-session'));
    await page.reload();
    await page.waitForLoadState('networkidle');
    await expect(page.locator('.login-picker h2')).toHaveText('Sign in');
    await expect(page.locator('input[type="file"]')).toBeAttached();
  });

  test('3. card gallery: seven known types and the unknown fallback render', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await page.waitForLoadState('networkidle');

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
    // An unrecognized type, and a known type at an unrecognized version.
    await expect(page.locator('.card-unknown')).toHaveCount(2);
  });

  test('4. card safety: a malicious payload yields no script, no request, literal text', async ({ page }) => {
    const externalRequests: string[] = [];
    page.on('request', req => {
      if (!req.url().includes(':7660')) externalRequests.push(req.url());
    });
    const consoleErrors: string[] = [];
    page.on('console', msg => {
      if (msg.type() === 'error') {
        const text = msg.text();
        if (!text.includes('Failed to load resource')) consoleErrors.push(text);
      }
    });

    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    const maliciousPayload =
      '<img src=x onerror=window.maliciousRan=true><script>window.maliciousScript=true</script>' +
      '<a href="javascript:window.maliciousLink=true">Click</a>';

    await page.evaluate((payload) => {
      const win = window as unknown as { RoymRegistry?: { renderCard: (c: unknown) => HTMLElement } };
      if (!win.RoymRegistry?.renderCard) {
        throw new Error('window.RoymRegistry.renderCard is not exposed by main.ts');
      }
      const container = document.createElement('div');
      container.id = 'safety-test-container';
      container.appendChild(
        win.RoymRegistry.renderCard({ type: 'request', version: 1, data: { summary: payload } }),
      );
      document.body.appendChild(container);
    }, maliciousPayload);

    await page.waitForTimeout(500);

    const maliciousRan = await page.evaluate(
      () => (window as any).maliciousRan || (window as any).maliciousScript,
    );
    expect(maliciousRan).toBeUndefined();
    expect(externalRequests).toEqual([]);
    expect(consoleErrors).toEqual([]);

    await expect(page.locator('#safety-test-container .card-request')).toHaveClass(/card-request/);
    const renderedText = await page.locator('#safety-test-container').innerText();
    expect(renderedText).toContain('<img src=x');
    expect(renderedText).toContain('<script>');
  });
});
