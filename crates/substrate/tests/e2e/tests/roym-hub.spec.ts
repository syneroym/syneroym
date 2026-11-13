import { test, expect, type Page } from '../fixtures';
import {
  addContact, addListingTranslation, addMessageLanguage, chooseHubLanguage, ensureOwnSynOrg,
  hubMessage, loginWithDelegatedKey, openConversation, openMessages, pickCategory, publishListing,
  rpcCall, saveProfile, sendMessage, threadMessage,
} from '../hub-helpers';
import { readE2EPorts } from '../ports';

/**
 * Hold every `/rpc` call to `method` instead of sending it to the node. `answer(i, result)`
 * then replies to the i-th held call, so a test chooses the order the answers arrive in.
 */
async function holdRpc(page: Page, method: string) {
  const held: { release: (result: unknown) => Promise<void> }[] = [];
  await page.route('**/rpc', async (route) => {
    const body = route.request().postDataJSON() as { method: string; id: number };
    if (body.method !== method) return route.fallback();
    held.push({
      release: (result) =>
        route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result }),
        }),
    });
  });
  return { held, answer: (i: number, result: unknown) => held[i].release(result) };
}

/**
 * Wait for the page to draw what a reply just set going. The reply is parsed and drawn within
 * milliseconds, and nothing on the page says when that is done, so this is a fixed wait that is
 * far longer than needed. A wait that is too short can only make a broken page look right.
 */
async function settle(page: Page) {
  await page.evaluate(() => new Promise((done) => setTimeout(done, 1_000)));
}

let HUB_URL: string;
let AUTH_ORIGIN: string;

test.describe('Roym Hub', () => {
  test.beforeAll(() => {
    const ports = readE2EPorts();
    HUB_URL = process.env.ROYM_HUB_URL || `http://127.0.0.1:${ports.gatewayPort}`;
    AUTH_ORIGIN = `http://auth.localhost:${ports.gatewayPort}`;
  });

  test.beforeEach(async ({ page }) => {
    expect(process.env.ROYM_WEB_ALIAS).toBeDefined();
  });

  test.describe('in a browser that asks for Hindi', () => {
    test.use({ locale: 'hi-IN' });

    test('the Hub opens in Hindi with no choice made', async ({ page }) => {
      await page.goto(HUB_URL);
      await page.waitForLoadState('networkidle');

      await expect(page.locator('html')).toHaveAttribute('lang', 'hi');
      await expect(page.locator('.login-picker h2')).toHaveText(hubMessage('hi', 'login-title'));
    });
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
    await page.waitForLoadState('domcontentloaded');
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

  test('2b. a late first load of a tab cannot draw a second copy of its screen', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    // Click Contacts twice while its first call to the node is held, then answer the calls
    // newest first. The older load finishes last, after the newer screen is already drawn.
    const contacts = await holdRpc(page, 'contacts.list');
    const button = page.getByRole('button', { name: 'Contacts', exact: true });
    await button.click();
    await button.click();
    await expect.poll(() => contacts.held.length).toBe(2);

    await contacts.answer(1, []);
    await expect(page.locator('.contacts-screen')).toHaveCount(1);
    await contacts.answer(0, []);
    await settle(page);

    await expect(page.locator('.contacts-screen')).toHaveCount(1);
    await expect(page.locator('.tab-content')).toHaveCount(1);
  });

  test('2c. two overlapping list reloads in one screen draw each row once', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await openMessages(page);

    // Clicking a search hit reloads the conversation list. Click it twice while the list
    // call is held, then answer newest first, so the older reload finishes last.
    await page.route('**/rpc', async (route) => {
      const { method, id } = route.request().postDataJSON() as { method: string; id: number };
      if (method !== 'conversation.search') return route.fallback();
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id, result: { matches: [{ conversation: 'c1', body: 'hello' }] } }),
      });
    });
    const list = await holdRpc(page, 'conversation.list');
    await page.locator('.message-search-input').fill('hello');
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    const hit = page.locator('.search-hit').first();
    await hit.click();
    await hit.click();
    await expect.poll(() => list.held.length).toBe(2);

    const rows = [{ id: 'c1', peer_address: 'peer-address', last_activity_ms: 0, message_count: 0 }];
    await list.answer(1, { conversations: rows });
    await expect(page.locator('.conversation-item')).toHaveCount(1);
    await list.answer(0, { conversations: rows });
    await settle(page);

    await expect(page.locator('.conversation-item')).toHaveCount(1);
  });

  test('2d. two quick clicks on Search list each hit once', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await openMessages(page);

    const search = await holdRpc(page, 'conversation.search');
    await page.locator('.message-search-input').fill('hello');
    const button = page.getByRole('button', { name: 'Search', exact: true });
    await button.click();
    await button.click();
    await expect.poll(() => search.held.length).toBe(2);

    const matches = { matches: [{ conversation: 'c1', body: 'hello there' }] };
    await search.answer(1, matches);
    await expect(page.locator('.search-hit')).toHaveCount(1);
    await search.answer(0, matches);
    await settle(page);

    await expect(page.locator('.search-hit')).toHaveCount(1);
  });

  test('3. card gallery: seven known types and the unknown fallback render', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await page.waitForLoadState('domcontentloaded');

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
    const authOrigin = AUTH_ORIGIN;
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

    await saveProfile(page, 'Alice In E2E', 'iroh://alice-e2e-test-address');
  });

  test('6. contacts tab: adding a contact displays it in contacts list', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await addContact(page, 'did:key:h7wybobtestdid123456789012345678901234567890', 'iroh://bob-e2e-test-address');
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
    await expect(page.locator('.backup-separate-note')).toContainText(
      'Each bundle below is signed by you when it is exported',
    );

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
    await page.waitForLoadState('domcontentloaded');
    await page.getByRole('button', { name: 'Listings' }).click();
    const row = page.locator('.listing-row', { hasText: 'Hedge trimming' });
    await expect(row).toHaveCount(1);
    await expect(row.locator('.listing-status')).toHaveText('active');
  });

  test('10. messages tab: a sent message shows pending, never "delivered" early, and its delete dialog claims nothing about the peer copy', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await openMessages(page);
    await openConversation(page, 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000000');
    await sendMessage(page, 'hello nobody');

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

  test('messages tab: a message in two languages keeps its text and has its main language', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await openMessages(page);
    await openConversation(page, 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000000');

    // The language list shows English by default
    const langRows = page.locator('.messages-language-row');
    await expect(langRows.first().locator('.messages-language-name')).toHaveText('English');

    await addMessageLanguage(page, 'hi');
    const hindiRow = langRows.nth(1);
    await expect(hindiRow.locator('.messages-language-name')).toHaveText('Hindi');
    await hindiRow.locator('.messages-language-move-up').click();

    await expect(langRows.first().locator('.messages-language-name')).toHaveText('Hindi');
    await expect(langRows.nth(1).locator('.messages-language-name')).toHaveText('English');

    const runTag = Date.now().toString(36);
    const msg = `नमस्ते, see you at 5 ${runTag}`;
    await sendMessage(page, msg);

    const body = page.locator('.thread-messages .message .message-body', { hasText: msg }).first();
    await expect(body).toBeVisible({ timeout: 15_000 });
    await expect(body).toHaveAttribute('lang', 'hi');
    await expect(body).toHaveText(msg);

    // No "Language not stated" note
    const note = page.locator('.thread-messages .message .message-language-note');
    await expect(note).toHaveCount(0);

    // Reload and reopen conversation: still lang="hi" read back from store
    await page.reload();
    await page.waitForLoadState('domcontentloaded');
    await openMessages(page);
    await openConversation(page, 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000000');

    const bodyReloaded = page.locator('.thread-messages .message .message-body', { hasText: msg }).first();
    await expect(bodyReloaded).toBeVisible({ timeout: 15_000 });
    await expect(bodyReloaded).toHaveAttribute('lang', 'hi');
    await expect(bodyReloaded).toHaveText(msg);
  });

  test('11. listings tab: a malicious listing title renders as literal text, no element, no request', async ({ page }) => {
    const externalRequests: string[] = [];
    const hubOrigin = new URL(HUB_URL).origin;
    const authOrigin = AUTH_ORIGIN;
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
    await page.waitForLoadState('domcontentloaded');
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
    await ensureOwnSynOrg(page, DIRECTORY_DID!, 'bicycle-repair', 'en');
    await publishListing(page, title, 'bicycle-repair', DIRECTORY_DID!, 'en');
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

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    const hit = page.locator('.search-hit', { hasText: 'Wheel truing service' });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    await expect(hit.locator('.hit-sources')).toContainText(DIRECTORY_DID!);
    await expect(hit.locator('.hit-age')).toContainText('ago');
    await expect(hit.locator('.evidence-revocation')).toHaveText('revocation: unknown');
    await expect(hit.locator('.evidence-membership')).toContainText('Member of');
    await expect(hit.locator('.evidence-membership')).toContainText('checked on your node');

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
      if (!url.startsWith(hubOrigin) && !url.startsWith(AUTH_ORIGIN)) {
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

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
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

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
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
      await pickCategory(page.locator('.synorg-categories'), 'bicycle-repair');
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

  test('21. Listings tab: a listing owner chooses a directory and publishes a listing to it, and a draft is refused with a reason', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    // A SynOrg must exist for a publication to land.
    await page.getByRole('button', { name: 'SynOrg', exact: true }).click();
    if ((await page.locator('.synorg-status').innerText()).includes('runs no SynOrg')) {
      await page.locator('.synorg-name').fill('E2E Trades');
      await pickCategory(page.locator('.synorg-categories'), 'bicycle-repair');
      await page.locator('.synorg-support').fill('help@example.org');
      await page.locator('.synorg-dispute').fill('Email support.');
      await page.locator('.synorg-retention-days').fill('30');
      await page.locator('.synorg-rules').fill('rules');
      await page.getByRole('button', { name: 'Save settings' }).click();
      await expect(page.locator('.synorg-save-status')).toHaveText('Saved.', { timeout: 15_000 });
      const whoami = await rpcCall(page, 'session.whoami', {});
      const issued = await rpcCall(page, 'credential.issue', {
        member_did: whoami.result?.did,
        categories: ['bicycle-repair'],
        expires_at_secs: Math.floor(Date.now() / 1000) + 365 * 24 * 3600,
      });
      expect(issued.result?.record_id).toBeTruthy();
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
    await page.locator('.listing-status-select').selectOption('draft');
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

  test('22. Directory tab: the results view shows the loopback hit and names the unreachable sources', async ({ page }) => {
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

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    // The loopback hit is shown while the unreachable sources are reported
    // as errors without replacing the results.
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

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    await expect(page.locator('.search-progress')).toContainText('8 directories searched', {
      timeout: 40_000,
    });

    await expect(page.locator('.search-hit', { hasText: 'Ceiling run listing' })).toHaveCount(1);
    // No 503 and no "busy" wording reached the person, and every source that
    // did not answer is shown as a directory problem, not this node's.
    const pageText = await page.innerText('body');
    expect(pageText).not.toContain('503');
    expect(pageText).not.toContain('This installation could not start the search');
    expect(pageText.toLowerCase()).not.toContain('this installation was busy');
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
    await expect(quoteCard).toContainText('Total: €45.00');
    await expect(quoteCard).toContainText('Payee, as agreed in this quote: Y Repairs');
    await expect(quoteCard).toContainText('Expires:');
    await expect(quoteCard.locator('.quote-expiry time')).toHaveAttribute('datetime', /^\d{4}-\d{2}-\d{2}T/);
    await expect(quoteCard).toContainText('Cancellation terms: 24 hours notice required for full refund');
    await expect(quoteCard).toContainText('Dispute path: Small claims court or informal mediation');

    const receiptCard = page.locator('.card-agreement-receipt');
    await expect(receiptCard).toContainText('Scope: Replace brake cable and adjust pads');
    await expect(receiptCard).toContainText('Total: €45.00');
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
    const authOrigin = AUTH_ORIGIN;
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
              where: 'at-receiver',
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

    await openMessages(page);
    await openConversation(page, 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000026');

    await page.locator('.toggle-request-form').click();
    await expect(page.locator('.request-form')).toBeVisible();

    const descText = 'Emergency gutter and downpipe cleaning';
    await page.locator('.request-desc-input').fill(descText);
    await pickCategory(page.locator('.request-cats-input'), 'home-cleaning');
    await pickCategory(page.locator('.request-cats-input'), 'deep-cleaning');
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

    await openMessages(page);
    await openConversation(page, 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000029');

    await page.locator('.toggle-request-form').click();
    await expect(page.locator('.request-form')).toBeVisible();
    const reqNotice = await page.locator('.request-form .data-use-notice').innerText();
    expect(reqNotice).toBe(
      'This request is signed by you and sent to the person or business you chose. They ' +
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
    await expect(quoteCard).toContainText('Total: €1,200.00');
    await expect(quoteCard).toContainText('Payee, as agreed in this quote: Timber Crafts Ltd');
    await expect(quoteCard).toContainText('Cancellation terms: Strict 48h cancellation');

    // The date is in the Hub language's own format, which Node and Chromium
    // may write a little differently, so check its parts. 1600000000 is
    // September 2020 in every time zone.
    await expect(quoteCard.locator('.quote-expiry')).toContainText('Expires: ');
    await expect(quoteCard.locator('.quote-expiry')).toContainText('2020');
    await expect(quoteCard.locator('.quote-expiry')).toContainText('(expired)');
    await expect(quoteCard.locator('.quote-expired-notice')).toContainText('This quote expired on ');
    await expect(quoteCard.locator('.quote-expired-notice')).toContainText('2020');
    await expect(quoteCard.locator('.quote-expired-notice')).toContainText('Ask for a new one.');

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
          counterparty_did: 'did:key:testme',
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
          counterparty_did: 'did:key:testme',
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

  test('40. messages tab: searching for a word from a sent message shows one hit, and clicking it opens that conversation', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await openMessages(page);
    const peerDid = 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000040';
    await openConversation(page, peerDid);
    const heading = page.locator('.conversation-thread h3').first();
    await expect(heading).toHaveText(peerDid, { timeout: 15_000 });

    // The node is shared across scenarios (and retries), so the searched
    // word is unique to this run: exactly one message can hold it.
    const word = `searchable${Date.now()}`;
    await sendMessage(page, `please trim the ${word} hedge`);

    // Switch the thread away, so the click below has something to change.
    const otherDid = 'did:key:z6MkhubE2ePeerAddressNeverAnswers0000000000040b';
    await page.locator('.open-conversation input').fill(otherDid);
    await page.getByRole('button', { name: 'Open conversation' }).click();
    await expect(heading).toHaveText(otherDid, { timeout: 15_000 });

    await page.locator('.message-search-input').fill(word);
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    const hits = page.locator('.search-results .search-hit');
    await expect(hits).toHaveCount(1, { timeout: 15_000 });
    await expect(hits.locator('.snippet')).toHaveText(`please trim the ${word} hedge`);

    await hits.click();
    await expect(heading).toHaveText(peerDid, { timeout: 15_000 });
  });

  test('41. messages tab: a message sent in one tab appears in a second open tab with no reload', async ({ context, page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    await openMessages(page);
    const peerDid = 'did:key:z6MkhubE2ePeerAddressNeverAnswers00000000000041';
    await openConversation(page, peerDid);

    // Open a second tab on the same Hub in the same browser context.
    const page2 = await context.newPage();

    // The second page routes the first /_syneroym/events request to return 503 once,
    // exercising the backoff and reconnect logic.
    let failedOnce = false;
    await page2.route('**/_syneroym/events*', async (route) => {
      if (!failedOnce) {
        failedOnce = true;
        await route.fulfill({
          status: 503,
          headers: { 'Retry-After': '1' },
          body: 'Service Unavailable',
        });
      } else {
        await route.continue();
      }
    });

    await page2.goto(HUB_URL);
    await page2.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page2);

    await openMessages(page2);
    await openConversation(page2, peerDid);

    // Send a message from page 1.
    const messageText = `live-sync-${Date.now()}`;
    await sendMessage(page, messageText);

    // On page 1, the message is visible.
    await expect(threadMessage(page, messageText)).toBeVisible({ timeout: 15_000 });

    // On page 2, the message appears with no reload.
    await expect(threadMessage(page2, messageText)).toBeVisible({ timeout: 15_000 });
    expect(failedOnce).toBe(true);

    await page2.close();
  });

  test('a category search finds the same listings in English and in Hindi', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Multilingual bike repair listing');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
    const enResponsePromise = page.waitForResponse(
      (res) => res.url().includes('/rpc') && res.request().postData()?.includes('directory.find') === true,
    );
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    const hitEn = page.locator('.search-hit', { hasText: 'Multilingual bike repair listing' });
    await expect(hitEn).toHaveCount(1, { timeout: 20_000 });
    const enResponse = await enResponsePromise;
    const enJson = (await enResponse.json()) as { result?: { hits?: Array<{ listing_id: string }> } };
    const englishListingIds = (enJson.result?.hits ?? []).map((h) => h.listing_id).sort();
    expect(englishListingIds.length).toBeGreaterThan(0);

    await chooseHubLanguage(page, 'hi');
    await page.getByRole('button', { name: hubMessage('hi', 'tab-directory') }).click();
    await pickCategory(page.locator('.search-categories'), 'bicycle-repair', 'hi');
    const hiResponsePromise = page.waitForResponse(
      (res) => res.url().includes('/rpc') && res.request().postData()?.includes('directory.find') === true,
    );
    await page.getByRole('button', { name: hubMessage('hi', 'directory-search-button'), exact: true }).click();

    const hitHi = page.locator('.search-hit', { hasText: 'Multilingual bike repair listing' });
    await expect(hitHi).toHaveCount(1, { timeout: 20_000 });
    const hiResponse = await hiResponsePromise;
    const hiJson = (await hiResponse.json()) as { result?: { hits?: Array<{ listing_id: string }> } };
    const hindiListingIds = (hiJson.result?.hits ?? []).map((h) => h.listing_id).sort();
    expect(hindiListingIds).toEqual(englishListingIds);

    const hitCategories = await hitHi.locator('.hit-categories').innerText();
    expect(hitCategories).toContain(hubMessage('hi', 'taxonomy-category-bicycle-repair'));

    await chooseHubLanguage(page, 'en');
  });

  test("an English reader sees the author's English title of a Hindi listing and can open the original", async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Bilingual seed listing');

    await page.getByRole('button', { name: 'Listings' }).click();
    await page.locator('.listing-text-lang').selectOption('hi');
    await page.locator('.listing-title-input').fill('झाड़ी कटाई');
    await page.locator('.listing-slug-input').fill('hedge-trimming');
    await pickCategory(page.locator('.listing-categories-input'), 'bicycle-repair');
    await page.locator('.listing-address-input').fill(DIRECTORY_DID!);
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('40');
    await addListingTranslation(page, 'en', 'Hedge trimming');
    await page.locator('.save-listing').click();
    const saveResult = page.locator('.listing-save-result');
    await expect(saveResult).toContainText('Saved lst_', { timeout: 15_000 });
    const listingIdMatch = (await saveResult.innerText()).match(/lst_[a-zA-Z0-9]+/);
    expect(listingIdMatch).toBeTruthy();
    const listingId = listingIdMatch![0];

    const row = page.locator('.listing-row', { hasText: 'झाड़ी कटाई' }).first();
    await row.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await row.locator('.publish-listing').click();
    await expect(row.locator('.publish-status')).toContainText('Published', { timeout: 15_000 });

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    await pickCategory(page.locator('.search-categories'), 'bicycle-repair');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    const hit = page.locator(`.search-hit[data-listing-id="${listingId}"]`);
    await expect(hit).toHaveCount(1, { timeout: 20_000 });

    const titleEl = hit.locator('.hit-title');
    await expect(titleEl).toHaveText('Hedge trimming');
    await expect(titleEl).toHaveAttribute('lang', 'en');
    await expect(hit.locator('.record-text-note')).toHaveText('Translated by the author from Hindi.');

    await hit.locator('.record-text-switch').click();
    await expect(titleEl).toHaveText('झाड़ी कटाई');
    await expect(titleEl).toHaveAttribute('lang', 'hi');
    await expect(hit.locator('.record-text-note')).toHaveText('Original text, in Hindi.');

    await chooseHubLanguage(page, 'hi');
    await page.getByRole('button', { name: hubMessage('hi', 'tab-directory') }).click();
    await pickCategory(page.locator('.search-categories'), 'bicycle-repair', 'hi');
    await page.getByRole('button', { name: hubMessage('hi', 'directory-search-button'), exact: true }).click();

    const hitHi = page.locator(`.search-hit[data-listing-id="${listingId}"]`);
    await expect(hitHi).toHaveCount(1, { timeout: 20_000 });
    const titleHi = hitHi.locator('.hit-title');
    await expect(titleHi).toHaveText('झाड़ी कटाई');
    await expect(titleHi).toHaveAttribute('lang', 'hi');
    await expect(hitHi.locator('.record-text-note')).toHaveText('मूल विवरण, हिन्दी में।');

    await chooseHubLanguage(page, 'en');
  });

  test('a Latin search finds a Devanagari listing, and the other way', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    await ensureSynOrgWithListing(page, 'Cross-script search seed');

    const tag1 = Date.now().toString(36);
    const tag2 = (Date.now() + 1).toString(36);
    const title1 = `नल वाला ${tag1}`;
    const title2 = `nal wala ${tag2}`;

    await publishListing(page, title1, 'bicycle-repair', DIRECTORY_DID!, 'en');
    await publishListing(page, title2, 'bicycle-repair', DIRECTORY_DID!, 'en', 'hi-Latn');

    await removeAllSources(page);
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });

    await page.locator('.search-text-input').fill(`nal wala ${tag1}`);
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    const hit1 = page.locator('.search-hit', { hasText: title1 });
    await expect(hit1).toHaveCount(1, { timeout: 20_000 });

    await page.locator('.search-text-input').fill(`नल वाला ${tag2}`);
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    const hit2 = page.locator('.search-hit', { hasText: title2 });
    await expect(hit2).toHaveCount(1, { timeout: 20_000 });
  });

  test('contact search finds a name in another script', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);

    const tag = Date.now().toString(36);
    const people = [
      { did: `did:key:z6MkuDeva${tag}`, addr: `conv_deva_${tag}`, name: `राम ${tag}` },
      { did: `did:key:z6MkuLatn${tag}`, addr: `conv_latn_${tag}`, name: `Ram ${tag}` },
      { did: `did:key:z6MkuOther${tag}`, addr: `conv_other_${tag}`, name: `Sita ${tag}` },
    ];
    for (const p of people) {
      await rpcCall(page, 'contacts.upsert', {
        person_did: p.did,
        conversation_address: p.addr,
        display_name: p.name,
      });
    }
    const [deva, latn, other] = people.map((p) => p.name);

    await page.getByRole('button', { name: 'Contacts' }).click();
    const screen = page.locator('.contacts-screen');
    await expect(screen).toBeVisible();
    // Before any search all three rows show, so a row that is gone later was filtered out.
    await expect(screen).toContainText(other);

    const searchInput = page.locator('.contacts-search-input');
    const searchBtn = page.locator('.contacts-search-button');

    // Show the whole list again and wait for it, so each search starts from a list that can
    // only change when the Hub gets the answer.
    const showAll = async () => {
      await searchInput.fill('');
      await searchBtn.click();
      await expect(screen).toContainText(other, { timeout: 15_000 });
      await expect(screen).toContainText(deva);
      await expect(screen).toContainText(latn);
    };

    for (const query of ['ram', 'राम']) {
      await showAll();
      await searchInput.fill(query);
      await searchBtn.click();
      await expect(screen).not.toContainText(other, { timeout: 15_000 });
      await expect(screen).toContainText(deva);
      await expect(screen).toContainText(latn);
    }

    await showAll();

    // With no search, the names in the Hub's own script (Latin in English) come first.
    const text = await screen.innerText();
    const idxLatn = text.indexOf(latn);
    const idxDeva = text.indexOf(deva);
    expect(idxLatn).toBeGreaterThanOrEqual(0);
    expect(idxDeva).toBeGreaterThanOrEqual(0);
    expect(idxLatn).toBeLessThan(idxDeva);
  });
});
