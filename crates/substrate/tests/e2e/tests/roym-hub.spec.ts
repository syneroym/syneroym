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

  test('8. backup tab: shows the five service bundles separately and says so', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Backup' }).click();
    await expect(page.locator('.backup-screen .bundle-row')).toHaveCount(5);
    await expect(page.locator('.backup-separate-note')).toContainText('five bundles below are exported separately today');

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

  // --- C6: the Directory and SynOrg surfaces -------------------------------

  const DIRECTORY_DID = process.env.ROYM_DIRECTORY_DID;
  // Six random-looking DIDs that resolve to nothing; used as unreachable
  // sources. `add-source` stores a source even when its probe fails.
  const bogusDid = (n: number) =>
    `did:key:h7wybogusdirectory${n}00000000000000000000000000000000`.slice(0, 56);

  /// Turn this node into a SynOrg (idempotent) and make sure one signed
  /// listing has been published to its own directory over the loopback
  /// path, so a search has something real to return.
  async function ensureSynOrgWithListing(page: Page, title: string) {
    expect(DIRECTORY_DID, 'ROYM_DIRECTORY_DID must be set by global-setup').toBeTruthy();
    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    const status = await page.locator('.synorg-status').innerText();
    if (status.includes('runs no SynOrg')) {
      await page.locator('.synorg-name').fill('E2E Trades');
      await page.locator('.synorg-rules').fill('Be honest. Show up.');
      await page.locator('.synorg-categories').fill('cycling');
      await page.locator('.synorg-support').fill('help@example.org');
      await page.locator('.synorg-dispute').fill('Email support.');
      await page.locator('.synorg-retention-days').fill('30');
      await page.getByRole('button', { name: 'Save settings' }).click();
      await expect(page.locator('.synorg-save-status')).toHaveText('Saved.', { timeout: 15_000 });
      await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    }
    // Every test shares this one node, so publications from earlier tests
    // sit in the 24 h ledger. Keep the limit generous so a fresh publish
    // does not hit it -- the one case that tests the limit lowers it again.
    await expect(page.locator('.limit-max')).toBeVisible({ timeout: 15_000 });
    await page.locator('.limit-window').fill('86400');
    await page.locator('.limit-max').fill('1000'); // the ceiling; 0 would block every publish
    await page.getByRole('button', { name: 'Save limit' }).click();
    await expect(page.locator('.limit-status')).toHaveText('Saved.', { timeout: 15_000 });

    await page.getByRole('button', { name: 'Listings' }).click();
    await page.locator('.listing-title-input').fill(title);
    await page.locator('.listing-categories-input').fill('cycling');
    await page.locator('.listing-address-input').fill(DIRECTORY_DID!);
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('40');
    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });

    const row = page.locator('.listing-row', { hasText: title }).first();
    await row.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await row.locator('.publish-listing').click();
    await expect(row.locator('.publish-status')).toContainText('Published', { timeout: 15_000 });
  }

  async function removeAllSources(page: Page) {
    await page.getByRole('button', { name: 'Directory', exact: true }).click();
    // Wait for the sources list to finish its first async render.
    await expect(page.locator('.directory-sources h3')).toBeVisible({ timeout: 15_000 });
    for (let i = 0; i < 12; i++) {
      const rows = page.locator('.directory-source-row');
      const n = await rows.count();
      if (n === 0) break;
      await rows.first().locator('.remove-source').click();
      await expect(page.locator('.directory-source-row')).toHaveCount(n - 1, { timeout: 10_000 });
    }
    await expect(page.locator('.directory-source-row')).toHaveCount(0);
  }

  test('13. Directory tab: adding a source by DID lists it; removing it empties the list', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await removeAllSources(page);

    const did = bogusDid(9);
    await page.locator('.add-source-did').fill(did);
    await page.locator('.add-source-label').fill('a friend gave me this');
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    const row = page.locator('.directory-source-row', { hasText: did });
    await expect(row).toHaveCount(1);
    await expect(row.locator('.source-label')).toHaveText('a friend gave me this');

    await row.locator('.remove-source').click();
    await expect(page.locator('.directory-source-row')).toHaveCount(0);
    await expect(page.locator('.directory-empty')).toBeVisible();
  });

  test('14. Directory tab: a result shows its source, age in words, and both unknowns as words, never bare "verified"', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Wheel truing service');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    await page.locator('.search-categories').fill('cycling');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    const hit = page.locator('.search-hit', { hasText: 'Wheel truing service' });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    await expect(hit.locator('.hit-sources')).toContainText(DIRECTORY_DID!);
    await expect(hit.locator('.hit-age')).toContainText('ago');
    await expect(hit.locator('.evidence-revocation')).toHaveText('revocation: unknown');
    await expect(hit.locator('.evidence-membership')).toHaveText('membership: not checked');

    // The evidence block never uses the bare word "verified" -- it says what
    // was and was not checked.
    const evidenceText = (await hit.locator('.hit-evidence').innerText()).toLowerCase();
    expect(evidenceText, `evidence was: ${evidenceText}`).not.toMatch(/\bverified\b/);
    expect(evidenceText).toContain('signature: checked on your node');

    // Refused evidence, when present, renders in its own block below the
    // results, never inside a result card.
    expect(await hit.locator('.refused-hit').count()).toBe(0);
  });

  test('16. Directory tab: a malicious listing title from a directory renders as literal text, no element, no request', async ({ page }) => {
    const externalRequests: string[] = [];
    const hubOrigin = new URL(HUB_URL).origin;
    page.on('request', (req) => {
      const url = req.url();
      if (!url.startsWith(hubOrigin) && !url.startsWith('http://auth.localhost:7660')) {
        externalRequests.push(url);
      }
    });

    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, '<img src=x onerror=window.dirEvil=true> bike wash');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    await page.locator('.search-categories').fill('cycling');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    const hit = page.locator('.search-hit', { hasText: '<img src=x' });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    expect(await hit.locator('.hit-title img').count()).toBe(0);
    expect(await page.evaluate(() => (window as any).dirEvil)).toBeUndefined();
    expect(externalRequests).toEqual([]);
  });

  test('17. Directory tab: a search with no sources shows the empty state that a directory is optional', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await removeAllSources(page);

    await page.getByRole('button', { name: 'Search', exact: true }).click();
    await expect(page.locator('.search-progress')).toContainText('not added any directories', {
      timeout: 15_000,
    });
    await expect(page.locator('.search-progress')).toContainText('direct link');
    await expect(page.locator('.search-results .no-results')).toBeVisible();
  });

  test('18. Directory tab: a source that errored shows its error beside it and the other sources still render', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Chain lube special');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    await page.locator('.add-source-did').fill(bogusDid(1));
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    await page.locator('.search-categories').fill('cycling');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    await expect(page.locator('.search-hit', { hasText: 'Chain lube special' })).toHaveCount(1, {
      timeout: 20_000,
    });
    // The failed source's error is shown, and it did not replace the page.
    await expect(page.locator('.search-source-errors .source-error-line')).toHaveCount(1, {
      timeout: 20_000,
    });
  });

  test('19. SynOrg tab: creating a SynOrg writes settings and the rules text renders as text, not markup', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    const status = await page.locator('.synorg-status').innerText();
    if (status.includes('runs no SynOrg')) {
      await page.locator('.synorg-name').fill('E2E Trades');
      await page.locator('.synorg-categories').fill('cycling');
      await page.locator('.synorg-support').fill('help@example.org');
      await page.locator('.synorg-dispute').fill('Email support.');
      await page.locator('.synorg-retention-days').fill('30');
    }
    await page.locator('.synorg-rules').fill('<b>bold</b> rule and <script>window.rulesEvil=1</script>');
    await page.getByRole('button', { name: 'Save settings' }).click();
    await expect(page.locator('.synorg-save-status')).toHaveText('Saved.', { timeout: 15_000 });

    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    const rules = page.locator('.synorg-rules');
    await expect(rules).toHaveValue(/<b>bold<\/b>/);
    expect(await page.evaluate(() => (window as any).rulesEvil)).toBeUndefined();
  });

  test('20. SynOrg tab: raising the publication limit from the Hub lets a refused publisher succeed in one flow', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Warm-up publication');

    // Clamp the limit to 1 from the Hub.
    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    await page.locator('.limit-window').fill('86400');
    await page.locator('.limit-max').fill('1');
    await page.getByRole('button', { name: 'Save limit' }).click();
    await expect(page.locator('.limit-status')).toHaveText('Saved.', { timeout: 15_000 });

    // A second listing is refused at publish, over the limit, with a reason.
    await page.getByRole('button', { name: 'Listings' }).click();
    await page.locator('.listing-title-input').fill('Over-the-limit listing');
    await page.locator('.listing-address-input').fill(DIRECTORY_DID!);
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('30');
    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });
    const row = page.locator('.listing-row', { hasText: 'Over-the-limit listing' });
    await row.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await row.locator('.publish-listing').click();
    await expect(row.locator('.publish-status')).toContainText('Not published', { timeout: 15_000 });
    await expect(row.locator('.publish-status')).toContainText('rate limit');

    // Raise the limit from the Hub, then the same publish succeeds -- one
    // flow. (A large ceiling, because every earlier test's publications also
    // sit in this shared node's 24 h window.)
    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    await page.locator('.limit-max').fill('1000'); // the ceiling; 0 would block every publish
    await page.getByRole('button', { name: 'Save limit' }).click();
    await expect(page.locator('.limit-status')).toHaveText('Saved.', { timeout: 15_000 });

    await page.getByRole('button', { name: 'Listings' }).click();
    const row2 = page.locator('.listing-row', { hasText: 'Over-the-limit listing' });
    await row2.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await row2.locator('.publish-listing').click();
    await expect(row2.locator('.publish-status')).toContainText('Published', { timeout: 15_000 });
  });

  test('21. Listings tab: a provider chooses a directory and publishes a listing to it, and a draft is refused with a reason', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    // A SynOrg must exist for a publication to land.
    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    if ((await page.locator('.synorg-status').innerText()).includes('runs no SynOrg')) {
      await page.locator('.synorg-name').fill('E2E Trades');
      await page.locator('.synorg-categories').fill('cycling');
      await page.locator('.synorg-support').fill('help@example.org');
      await page.locator('.synorg-dispute').fill('Email support.');
      await page.locator('.synorg-retention-days').fill('30');
      await page.locator('.synorg-rules').fill('rules');
      await page.getByRole('button', { name: 'Save settings' }).click();
      await expect(page.locator('.synorg-save-status')).toHaveText('Saved.', { timeout: 15_000 });
    }

    await page.getByRole('button', { name: 'Listings' }).click();

    // An active listing publishes.
    await page.locator('.listing-title-input').fill('Journey step S7 listing');
    await page.locator('.listing-address-input').fill(DIRECTORY_DID!);
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('25');
    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });
    const active = page.locator('.listing-row', { hasText: 'Journey step S7 listing' });
    await active.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await active.locator('.publish-listing').click();
    await expect(active.locator('.publish-status')).toContainText('Published', { timeout: 15_000 });

    // A draft listing is refused at the directory with a reason.
    await page.locator('.listing-title-input').fill('Draft that cannot publish');
    await page.locator('.listing-address-input').fill(DIRECTORY_DID!);
    await page.locator('.listing-editor select').first().selectOption('draft');
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('25');
    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });
    const draft = page.locator('.listing-row', { hasText: 'Draft that cannot publish' });
    await draft.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await draft.locator('.publish-listing').click();
    await expect(draft.locator('.publish-status')).toContainText('Not published', { timeout: 15_000 });
    await expect(draft.locator('.publish-status')).toContainText('draft');
  });

  test('22. Directory tab: the results view fills in per source and a slow source does not blank the page', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Progressive render listing');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    for (const n of [1, 2, 3]) {
      await page.locator('.add-source-did').fill(bogusDid(n));
      await page.getByRole('button', { name: 'Add directory' }).click();
      await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    }

    await page.locator('.search-categories').fill('cycling');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    // The results view is re-merged and re-rendered as each source answers
    // (the client calls `directory.merge` per source, not only at the end),
    // and the loopback hit survives every re-render while the unreachable
    // sources are still being tried.
    const hit = page.locator('.search-hit', { hasText: 'Progressive render listing' });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    await expect(page.locator('.search-progress')).toContainText('4 directories searched', {
      timeout: 20_000,
    });
    await expect(hit).toHaveCount(1);
    // The unreachable sources are reported and did not replace the results.
    expect(await page.locator('.search-source-errors .source-error-line').count()).toBeGreaterThan(0);
  });

  test('23. Directory tab: a run at the full MAX_SOURCES completes with no 503 reaching the person', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Ceiling run listing');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    for (const n of [1, 2, 3, 4, 5, 6, 7] as const) {
      await page.locator('.add-source-did').fill(bogusDid(n));
      await page.getByRole('button', { name: 'Add directory' }).click();
      await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    }

    await page.locator('.search-categories').fill('cycling');
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    await expect(page.locator('.search-progress')).toContainText('8 directories searched', {
      timeout: 40_000,
    });

    await expect(page.locator('.search-hit', { hasText: 'Ceiling run listing' })).toHaveCount(1);
    // No 503 and no "busy" wording reached the person, and every source that
    // did not answer is shown as a directory problem, not this node's.
    const errorsText = await page.locator('.search-source-errors').innerText();
    expect(errorsText).not.toContain('503');
    expect(errorsText.toLowerCase()).not.toContain('this installation was busy');
  });

  // The deterministic NotStarted-vs-TimedOut mapping is proven by the
  // vitest suite; this case only asserts that a run at full concurrency
  // never blames the loopback source for a timeout and never conflates
  // the two outcome kinds.
  test('23b. Directory tab: a run at full concurrency never blames the loopback source for a timeout', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'NotStarted listing');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    for (const n of [1, 2, 3, 4, 5, 6, 7] as const) {
      await page.locator('.add-source-did').fill(bogusDid(n));
      await page.getByRole('button', { name: 'Add directory' }).click();
      await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    }

    // Drive the real client loop with `ignoreConcurrency` so it
    // oversubscribes this node's four-permit guest-HTTP admission door.
    const result = await page.evaluate(async () => {
      const win = window as unknown as {
        RoymDirectory: { runSearch: (q: unknown, o: unknown) => Promise<{ outcomes: Array<{ source: string; kind: string }> }> };
      };
      const { outcomes } = await win.RoymDirectory.runSearch(
        { categories: ['cycling'] },
        { ignoreConcurrency: true },
      );
      return outcomes;
    });

    const timedOut = result.filter((o) => o.kind === 'timed-out');
    // Whether or not a real 503 fired this run (it is scheduling-dependent),
    // the loopback source must never be blamed as timed out, and no outcome
    // conflates "this node was busy" with "the directory did not answer" --
    // `search.ts` keeps the 503 -> not-started mapping separate, proven
    // deterministically by the vitest suite.
    for (const o of timedOut) {
      expect(o.source).not.toBe(process.env.ROYM_DIRECTORY_DID);
    }
    expect(result.every((o) => ['ok', 'not-started', 'timed-out', 'not-found', 'refused', 'unreadable'].includes(o.kind))).toBe(true);
    // The node kept no `last_error` for a source it never actually called.
    const sources = await page.evaluate(async () => {
      const res = await fetch('/rpc', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(window as any).RoymSession.authHeaders() },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'directory.sources', params: {} }),
      });
      return (await res.json()).result.sources as Array<{ did: string; last_error?: { kind: string } }>;
    });
    const loopback = sources.find((s) => s.did === process.env.ROYM_DIRECTORY_DID);
    expect(loopback?.last_error ?? null).toBeNull();
  });

  test('24. components tab: request, quote, and agreement-receipt samples render real fields as text', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await page.getByRole('button', { name: 'Components' }).click();

    const requestCard = page.locator('.card-request');
    await expect(requestCard).toContainText('Front brake cable replacement and tuning');
    await expect(requestCard).toContainText('cycling, repair');

    const quoteCard = page.locator('.card-quote');
    await expect(quoteCard).toContainText('Scope: Replace brake cable and adjust pads');
    await expect(quoteCard).toContainText('Total: 45.00 EUR');
    await expect(quoteCard).toContainText('Payee, as agreed in this quote: Y Repairs');
    await expect(quoteCard).toContainText('Expires:');
    await expect(quoteCard).toContainText('Cancellation terms: 24 hours notice required for full refund');
    await expect(quoteCard).toContainText('Dispute path: Small claims court or informal mediation');

    const receiptCard = page.locator('.card-agreement-receipt');
    await expect(receiptCard).toContainText('Scope: Replace brake cable and adjust pads');
    await expect(receiptCard).toContainText('Total: 45.00 EUR');
    await expect(receiptCard).toContainText('Payee, as agreed in this quote: Y Repairs');
    await expect(receiptCard).toContainText('Both parties have signed these terms.');
    await expect(receiptCard).toContainText('Cancellation terms: 24 hours notice required for full refund');
    await expect(receiptCard).toContainText('Dispute path: Small claims court or informal mediation');
    const receiptText = await receiptCard.innerText();
    expect(receiptText.toLowerCase()).not.toContain('verified');
  });

  test('25. card safety on real templates: markup and javascript payee yield literal text, no element, no request', async ({ page }) => {
    const externalRequests: string[] = [];
    const hubOrigin = new URL(HUB_URL).origin;
    const authOrigin = 'http://auth.localhost:7660';
    page.on('request', (req) => {
      const url = req.url();
      if (!url.startsWith(hubOrigin) && !url.startsWith(authOrigin)) {
        externalRequests.push(url);
      }
    });
    const consoleErrors: string[] = [];
    page.on('console', (msg) => {
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
      const win = window as any;
      if (!win.RoymRegistry?.renderCard) {
        throw new Error('window.RoymRegistry.renderCard is not exposed by main.ts');
      }
      const container = document.createElement('div');
      container.id = 'real-template-safety-container';

      const quoteEl = win.RoymRegistry.renderCard({
        type: 'quote',
        version: 1,
        data: {
          terms: {
            scope: payload,
            currency: 'USD',
            amount_minor: 1000,
            payee: 'javascript:window.maliciousPayee=true',
            cancellation_terms: payload,
            refund_terms: payload,
            dispute_path: 'javascript:window.maliciousDispute=true',
            location: {
              where: 'at-customer',
              address: payload,
            },
          },
        },
      });

      const receiptEl = win.RoymRegistry.renderCard({
        type: 'agreement-receipt',
        version: 1,
        data: {
          terms: {
            scope: payload,
            currency: 'USD',
            amount_minor: 1000,
            payee: 'javascript:window.maliciousPayee=true',
            cancellation_terms: payload,
            refund_terms: payload,
            dispute_path: 'javascript:window.maliciousDispute=true',
          },
        },
      });

      container.appendChild(quoteEl);
      container.appendChild(receiptEl);
      document.body.appendChild(container);
    }, maliciousPayload);

    await page.waitForTimeout(500);

    const maliciousRan = await page.evaluate(
      () =>
        (window as any).maliciousRan ||
        (window as any).maliciousScript ||
        (window as any).maliciousLink ||
        (window as any).maliciousPayee ||
        (window as any).maliciousDispute,
    );
    expect(maliciousRan).toBeUndefined();
    expect(externalRequests).toEqual([]);
    expect(consoleErrors).toEqual([]);

    const container = page.locator('#real-template-safety-container');
    await expect(container.locator('img')).toHaveCount(0);
    await expect(container.locator('script')).toHaveCount(0);
    const anchors = container.locator('a');
    const count = await anchors.count();
    for (let i = 0; i < count; i++) {
      const href = await anchors.nth(i).getAttribute('href');
      expect(href?.toLowerCase().startsWith('javascript:')).toBe(false);
    }

    const text = await container.innerText();
    expect(text).toContain('<img src=x');
    expect(text).toContain('<script>');
  });

  test('26. messages tab: sending a request posts a card rendered as .card-request, never raw JSON', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Messages' }).click();
    const peerDid = 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000026';
    await page.locator('.open-conversation input').fill(peerDid);
    await page.getByRole('button', { name: 'Open conversation' }).click();

    await expect(page.locator('.conversation-thread h3').first()).toBeVisible({ timeout: 15_000 });

    await page.locator('.toggle-request-form').click();
    await expect(page.locator('.request-form')).toBeVisible();

    const descText = 'Emergency gutter and downpipe cleaning';
    await page.locator('.request-desc-input').fill(descText);
    await page.locator('.request-cats-input').fill('gutters, cleaning');
    await page.locator('.send-request-button').click();

    await expect(page.locator('.request-form')).not.toBeVisible({ timeout: 15_000 });

    const requestCard = page.locator('.thread-messages .card-request').first();
    await expect(requestCard).toBeVisible({ timeout: 15_000 });
    await expect(requestCard.locator('.request-description')).toHaveText(descText);

    const threadText = await page.locator('.thread-messages').innerText();
    expect(threadText).not.toContain('"card_type"');
    expect(threadText).not.toContain('"application/vnd.roym.card+json"');
  });

  test('27. messages tab: a refused card renders as .card-refused with data-verified="false" and no engage affordance', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.evaluate(() => {
      const win = window as any;
      if (!win.RoymRegistry?.renderRefusedCard) {
        throw new Error('window.RoymRegistry.renderRefusedCard is not exposed');
      }
      const container = document.createElement('div');
      container.id = 'refused-card-test-container';
      const card = win.RoymRegistry.renderRefusedCard('quote', 1, 'envelope signature tampered');
      container.appendChild(card);
      document.body.appendChild(container);
    });

    const refused = page.locator('#refused-card-test-container .card-refused');
    await expect(refused).toBeVisible();
    await expect(refused).toHaveAttribute('data-verified', 'false');
    await expect(refused).toContainText('Unverified quote card (v1)');
    await expect(refused).toContainText('envelope signature tampered');
    await expect(refused.locator('button')).toHaveCount(0);
    await expect(refused.locator('input')).toHaveCount(0);
    await expect(refused.locator('select')).toHaveCount(0);
    await expect(refused.locator('a')).toHaveCount(0);
  });

  test('29. messages tab: request form and quote form show notices pinned character-for-character', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await page.getByRole('button', { name: 'Messages' }).click();
    await page.locator('.open-conversation input').fill('did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000029');
    await page.getByRole('button', { name: 'Open conversation' }).click();

    await expect(page.locator('.conversation-thread h3').first()).toBeVisible({ timeout: 15_000 });

    await page.locator('.toggle-request-form').click();
    await expect(page.locator('.request-form')).toBeVisible();
    const reqNotice = await page.locator('.request-form .data-use-notice').innerText();
    expect(reqNotice).toBe(
      'This request is signed by you and sent to the provider you chose. They ' +
        'keep a copy. It carries the area you gave, not your exact address; an ' +
        'address is disclosed only inside a quote you accept.',
    );

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'quote-form-test-host';
      document.body.appendChild(host);
      const dummyReqCard = {
        message_id: 'msg_req_test',
        conversation: 'conv_test',
        direction: 'incoming',
        sender_timestamp_ms: 1000,
        card_type: 'request',
        version: 1,
        known: true,
        verified: true,
        expired: false,
        stored_at_secs: 1000,
      };
      win.RoymMessages.openQuoteForm(host, dummyReqCard, 'conv_test', async () => {});
    });

    const quoteNotice = await page.locator('#quote-form-test-host .address-disclosure-notice').innerText();
    expect(quoteNotice).toBe(
      'This address becomes part of a signed record that both parties keep ' +
        'and can export. It cannot be removed from a record already signed.',
    );
  });

  test('30. messages tab: an expired quote card renders full terms with no accept or decline button and names expiry date', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await page.evaluate(() => {
      const win = window as any;
      const container = document.createElement('div');
      container.id = 'expired-quote-test-container';

      const card = win.RoymRegistry.renderCard({
        type: 'quote',
        version: 1,
        data: {
          expired: true,
          quote_expires_at_secs: 1600000000,
          terms: {
            scope: 'Rebuild wooden front fence',
            currency: 'EUR',
            amount_minor: 120000,
            payee: 'Timber Crafts Ltd',
            cancellation_terms: 'Strict 48h cancellation',
            refund_terms: 'Materials non-refundable',
            quote_expires_at_secs: 1600000000,
          },
        },
      });
      container.appendChild(card);
      document.body.appendChild(container);
    });

    const quoteCard = page.locator('#expired-quote-test-container .card-quote');
    await expect(quoteCard).toBeVisible();

    await expect(quoteCard).toContainText('Scope: Rebuild wooden front fence');
    await expect(quoteCard).toContainText('Total: 1200.00 EUR');
    await expect(quoteCard).toContainText('Payee, as agreed in this quote: Timber Crafts Ltd');
    await expect(quoteCard).toContainText('Cancellation terms: Strict 48h cancellation');

    const expDate = new Date(1600000000 * 1000).toISOString();
    await expect(quoteCard.locator('.quote-expiry')).toContainText(`Expires: ${expDate} (expired)`);
    await expect(quoteCard.locator('.quote-expired-notice')).toContainText(
      `This quote expired on ${expDate}. Ask for a new one.`,
    );

    await expect(quoteCard.locator('.accept-quote-button')).toHaveCount(0);
    await expect(quoteCard.locator('.decline-quote-button')).toHaveCount(0);
  });

  test('31. declining a quote shows the "the other side is not told" sentence and hides accept', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'decline-action-test-host';
      document.body.appendChild(host);

      const activeQuoteCard = {
        message_id: 'msg_quote_active',
        conversation: 'conv_test',
        direction: 'incoming',
        sender_timestamp_ms: 1000,
        card_type: 'quote',
        version: 1,
        known: true,
        verified: true,
        expired: false,
        declined: false,
        record_id: 'rec_quote_active',
        stored_at_secs: 1000,
        data: {
          consumer_did: 'did:key:testme',
          terms: {
            scope: 'Tree trimming',
            currency: 'USD',
            amount_minor: 5000,
            payee: 'Gardener',
          },
        },
      };

      win.RoymMessages.renderCardActions(host, activeQuoteCard, 'conv_test', 'did:key:testme', async () => {});
    });

    const host = page.locator('#decline-action-test-host');
    await expect(host.locator('.accept-quote-button')).toBeVisible();
    await expect(host.locator('.decline-quote-button')).toBeVisible();

    await host.locator('.decline-quote-button').click();
    await expect(host.locator('.decline-dialog')).toBeVisible();

    const declineNote = await host.locator('.decline-dialog .decline-note').innerText();
    expect(declineNote).toBe(
      'This only changes what you see. The other side is not told, and no ' +
        'record is signed. Send them a message if you want them to know.',
    );

    await page.evaluate(() => {
      const win = window as any;
      const host = document.getElementById('decline-action-test-host')!;
      host.replaceChildren();
      const declinedQuoteCard = {
        message_id: 'msg_quote_declined',
        conversation: 'conv_test',
        direction: 'incoming',
        sender_timestamp_ms: 1000,
        card_type: 'quote',
        version: 1,
        known: true,
        verified: true,
        expired: false,
        declined: true,
        record_id: 'rec_quote_declined',
        stored_at_secs: 1000,
        data: {
          consumer_did: 'did:key:testme',
        },
      };
      win.RoymMessages.renderCardActions(host, declinedQuoteCard, 'conv_test', 'did:key:testme', async () => {});
    });

    await expect(host.locator('.accept-quote-button')).toHaveCount(0);
    await expect(host.locator('.decline-quote-button')).toHaveCount(0);
  });

  test('32. an installation with transaction unenrolled shows setup gate naming the missing service', async ({ page }) => {
    await page.route('**/rpc', async (route) => {
      const req = route.request();
      const postData = req.postData();
      if (postData && postData.includes('"transaction.signing-status"')) {
        const id = JSON.parse(postData).id;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id,
            result: { active: false },
          }),
        });
        return;
      }
      await route.continue();
    });

    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await expect(page.locator('.setup-screen')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.setup-screen h2')).toHaveText('Signing Certificate Required');
    const pendingText = await page.locator('.pending-services-text').innerText();
    expect(pendingText).toContain('transaction');

    await expect(page.locator('.tab-nav')).toHaveCount(0);
  });
});
