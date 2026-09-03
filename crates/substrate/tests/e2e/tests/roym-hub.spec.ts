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

    // Verify POST /rpc session.whoami with the stored bearer token returns
    // the delegated person DID matching the session bar.
    const whoamiResult = await page.evaluate(async () => {
      const win = window as unknown as {
        RoymSession?: { authHeaders: () => Record<string, string> };
      };
      const headers = win.RoymSession?.authHeaders() ?? {};
      const res = await fetch('/rpc', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...headers,
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'session.whoami',
          params: {},
        }),
      });
      return await res.json();
    });
    expect(whoamiResult.result?.auth).toBe('delegated');
    const whoamiDid = whoamiResult.result?.did;
    expect(whoamiDid).toBeTruthy();
    const sessionBarText = await page.locator('.session-bar').textContent();
    expect(sessionBarText).toContain(whoamiDid);
  });

  test('2. session survives a reload; clearing browser state returns to login', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    // A plain reload keeps the session (sessionStorage bearer token rides along).
    await page.reload();
    await page.waitForLoadState('networkidle');
    await expect(page.locator('.session-bar')).toContainText('did:key:', { timeout: 15_000 });

    // Clearing the session token and the stored key drops the session, so
    // the Hub is back to a clean "Sign in" screen -- an ordinary state, not
    // an error.
    await page.evaluate(() => {
      sessionStorage.clear();
      indexedDB.deleteDatabase('roym-hub-session');
    });
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

    await page.getByRole('button', { name: 'Components' }).click();

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
    const hubOrigin = new URL(HUB_URL).origin;
    const authOrigin = 'http://auth.localhost:7660';
    page.on('request', req => {
      const url = req.url();
      if (!url.startsWith(hubOrigin) && !url.startsWith(authOrigin)) {
        externalRequests.push(url);
      }
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

  test('5. profile tab: saving display name updates profile and displays signed record ID', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Profile', exact: true }).click();
    // Fill display name (first text input) and conversation address (second text input)
    const textInputs = page.locator('.profile-screen input[type="text"]');
    await textInputs.nth(0).fill('Alice In E2E');
    await textInputs.nth(1).fill('iroh://alice-e2e-test-address');
    await page.locator('.profile-screen button[type="submit"]').click();

    await expect(page.locator('.profile-screen p')).toContainText('Saved record rec_', { timeout: 15_000 });
  });

  test('6. contacts tab: adding a contact displays it in contacts list', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Contacts' }).click();
    const bobDid = 'did:key:h7wybobtestdid123456789012345678901234567890';
    // The contacts screen has: DID input (first), conversation address input (second)
    const contactInputs = page.locator('.contacts-screen input');
    await contactInputs.nth(0).fill(bobDid);
    await contactInputs.nth(1).fill('iroh://bob-e2e-test-address');
    await page.getByRole('button', { name: 'Upsert Contact' }).click();

    await expect(page.locator('.contacts-screen')).toContainText(bobDid, { timeout: 15_000 });
  });

  test('7. safety tab: blocking a DID displays it in block list', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Safety' }).click();
    const spammerDid = 'did:key:h7wyspammerdid123456789012345678901234567890';
    await page.locator('.safety-screen .block-input').fill(spammerDid);
    await page.getByRole('button', { name: 'Block' }).click();

    await expect(page.locator('.safety-screen')).toContainText(`Blocked: ${spammerDid}`);
  });

  test('8. backup tab: shows the three service bundles separately and says so', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Backup' }).click();
    await expect(page.locator('.backup-screen .bundle-row')).toHaveCount(3);
    await expect(page.locator('.backup-separate-note')).toContainText('exported separately today');

    // Each bundle exports on its own.
    const downloadPromise = page.waitForEvent('download');
    await page.locator('.bundle-row[data-export="conversation.export"] .bundle-export').click();
    const download = await downloadPromise;
    expect(download.suggestedFilename()).toBe('roym-conversation-bundle.json');
  });

  test('9. listings tab: a listing with three blocks round-trips and the editor sends no decimal', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    // Capture the listing.set request body so we can assert it carries no
    // decimal number -- the host refuses one before it signs.
    const setBodies: string[] = [];
    page.on('request', (req) => {
      if (req.method() === 'POST' && req.url().endsWith('/rpc')) {
        const body = req.postData() || '';
        if (body.includes('"listing.set"')) setBodies.push(body);
      }
    });

    await page.getByRole('button', { name: 'Listings' }).click();
    await page.locator('.listing-title-input').fill('Hedge trimming');
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('35.50');
    // booking + service blocks on (their defaults are already valid)
    await page.locator('.block-booking .block-enabled').check();
    await page.locator('.block-service .block-enabled').check();

    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });

    expect(setBodies.length).toBeGreaterThan(0);
    for (const body of setBodies) {
      // The amount became integer minor units, and no number anywhere in
      // the params is fractional (the host refuses a non-integer before it
      // signs). The JSON-RPC envelope's own "2.0" is not part of params.
      const params = JSON.parse(body).params as unknown;
      const everyNumberIsInteger = (v: unknown): boolean => {
        if (typeof v === 'number') return Number.isInteger(v);
        if (Array.isArray(v)) return v.every(everyNumberIsInteger);
        if (v && typeof v === 'object') return Object.values(v).every(everyNumberIsInteger);
        return true;
      };
      expect(everyNumberIsInteger(params)).toBe(true);
      expect((params as { payment: { amount_minor: number } }).payment.amount_minor).toBe(3550);
    }

    // Reload: the listing is still there with its title and status.
    await page.reload();
    await page.waitForLoadState('networkidle');
    await page.getByRole('button', { name: 'Listings' }).click();
    const row = page.locator('.listing-row', { hasText: 'Hedge trimming' });
    await expect(row).toHaveCount(1);
    await expect(row.locator('.listing-status')).toHaveText('active');
  });

  test('10. messages tab: a sent message shows pending, never "delivered" early, and its delete dialog claims nothing about the peer copy', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Messages' }).click();
    await page.locator('.open-conversation input').fill('did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000000');
    await page.getByRole('button', { name: 'Open conversation' }).click();

    await expect(page.locator('.conversation-thread h3')).toBeVisible({ timeout: 15_000 });
    await page.locator('.compose-input').fill('hello nobody');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    // The compose box clears only when the send succeeds.
    await expect(page.locator('.compose-input')).toHaveValue('', { timeout: 15_000 });

    const state = page.locator('.thread-messages .message .message-state').first();
    await expect(state).toHaveText('pending', { timeout: 15_000 });

    // The word "delivered" must not appear in the thread while it is pending.
    const threadText = await page.locator('.thread-messages').innerText();
    expect(threadText.toLowerCase()).not.toContain('delivered');

    // The delete dialog shows the service's note verbatim and claims
    // nothing about the peer's copy.
    await page.locator('.thread-messages .message .delete-message').first().click();
    const note = await page.locator('.delete-dialog .delete-note').innerText();
    expect(note).toBe(
      'The local copy is removed and a deletion record kept. A request to ' +
        'delete it was sent to the other side; whether their client honours it ' +
        "is theirs to decide, and this cannot check. This installation's own " +
        'message store still holds what it received.',
    );
    // The dialog must not claim the other party's copy is removed.
    expect(note.toLowerCase()).not.toContain('their copy is removed');
    expect(note.toLowerCase()).not.toContain('deleted for everyone');
    // The "also ask them" choice is present because this is a message we sent.
    await expect(page.locator('.delete-dialog .ask-peer')).toBeVisible();
  });

  test('11. listings tab: a malicious listing title renders as literal text, no element, no request', async ({ page }) => {
    const externalRequests: string[] = [];
    const hubOrigin = new URL(HUB_URL).origin;
    const authOrigin = 'http://auth.localhost:7660';
    page.on('request', (req) => {
      const url = req.url();
      if (!url.startsWith(hubOrigin) && !url.startsWith(authOrigin)) externalRequests.push(url);
    });

    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Listings' }).click();
    const evil = '<img src=x onerror=window.listingEvil=true>';
    await page.locator('.listing-title-input').fill(`${evil} cleaning`);
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('10');
    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });

    await page.reload();
    await page.waitForLoadState('networkidle');
    await page.getByRole('button', { name: 'Listings' }).click();

    const evilRow = page.locator('.listing-row', { hasText: '<img src=x' });
    await expect(evilRow).toHaveCount(1);
    expect(await evilRow.locator('.listing-title img').count()).toBe(0);
    expect(await page.evaluate(() => (window as any).listingEvil)).toBeUndefined();
    expect(externalRequests).toEqual([]);
  });

  test('12. safety tab: files a report and edits the first-contact limit', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Safety' }).click();

    await page.locator('.report-form input').fill('did:key:z6MkhubE2eReportedPerson000000000000000000000000');
    await page.locator('.report-form select').nth(1).selectOption('harassment');
    await page.getByRole('button', { name: 'File report' }).click();
    await expect(page.locator('.report-status')).toContainText('Recorded rep_', { timeout: 15_000 });
    await expect(page.locator('.report-list')).toContainText('harassment');

    await page.locator('.contact-limit-editor input').nth(0).fill('7200');
    await page.locator('.contact-limit-editor input').nth(1).fill('4');
    await page.getByRole('button', { name: 'Save limit' }).click();
    await expect(page.locator('.contact-limit-status')).toHaveText('Saved.', { timeout: 15_000 });
  });
});
