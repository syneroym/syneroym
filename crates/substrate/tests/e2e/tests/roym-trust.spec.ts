import { test, expect, type Page } from '../fixtures';
import { loginWithDelegatedKey, rpcCall } from '../hub-helpers';
import { readE2EPorts } from '../ports';

// The trust half of the Hub, on the one node the whole suite shares: the
// node's own directory is the loopback source, and its owner is at once the
// group's owner (who issues, suspends and revokes) and the provider (whose
// listing is published, hidden and shown). Each test leaves the next one's
// starting point behind, so they run in order and stop at the first failure.
test.describe.configure({ mode: 'serial' });

const NO_INSTANT_REMOVAL_NOTICE =
  "A group's decision reaches copies other people already hold only when they next check. " +
  'Nobody can promise it is removed everywhere at once.';
const LISTING_TITLE = 'Trust spec chain service';

test.describe('Roym Hub: cross-installation trust', () => {
  let HUB_URL: string;
  const DIRECTORY_DID = process.env.ROYM_DIRECTORY_DID;

  test.beforeAll(() => {
    HUB_URL = process.env.ROYM_HUB_URL || `http://127.0.0.1:${readE2EPorts().gatewayPort}`;
    expect(DIRECTORY_DID, 'ROYM_DIRECTORY_DID must be set by global-setup').toBeTruthy();
  });

  async function signIn(page: Page): Promise<string> {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    const whoami = await rpcCall(page, 'session.whoami', {});
    expect(whoami.result?.did).toBeTruthy();
    return whoami.result.did as string;
  }

  /// The SynOrg screen, with a SynOrg created first when this node runs none.
  async function openSynOrg(page: Page) {
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
    await expect(page.locator('.synorg-members')).toBeVisible({ timeout: 15_000 });
    // Every test shares this node's 24 h publication ledger.
    await page.locator('.limit-window').fill('86400');
    await page.locator('.limit-max').fill('1000');
    await page.getByRole('button', { name: 'Save limit' }).click();
    await expect(page.locator('.limit-status')).toHaveText('Saved.', { timeout: 15_000 });
  }

  function currentCredential(page: Page, did: string) {
    return page.locator(`.credential-row[data-status="current"][data-member="${did}"]`);
  }

  async function issueCredential(page: Page, did: string) {
    await page.locator('.issue-member-did').fill(did);
    await page.locator('.issue-days').fill('30');
    await page.locator('.issue-credential').click();
    await expect(page.locator('.issue-status')).toHaveText('Issued.', { timeout: 15_000 });
    await expect(currentCredential(page, did)).toHaveCount(1);
  }

  async function revokeCurrentCredential(page: Page, did: string) {
    const row = currentCredential(page, did);
    await row.locator('.revoke-reason').fill('spec: left the group');
    await row.locator('.revoke-credential').click();
    await expect(currentCredential(page, did)).toHaveCount(0, { timeout: 15_000 });
  }

  async function ensureListing(page: Page) {
    await page.getByRole('button', { name: 'Listings' }).click();
    await expect(page.locator('.listing-title-input')).toBeVisible({ timeout: 15_000 });
    if ((await page.locator('.listing-row', { hasText: LISTING_TITLE }).count()) > 0) return;
    await page.locator('.listing-title-input').fill(LISTING_TITLE);
    await page.locator('.listing-categories-input').fill('cycling');
    await page.locator('.listing-address-input').fill(DIRECTORY_DID!);
    await page.locator('.block-payment .block-enabled').check();
    await page.locator('.payment-amount-input').fill('40');
    await page.locator('.save-listing').click();
    await expect(page.locator('.listing-save-result')).toContainText('Saved lst_', { timeout: 15_000 });
  }

  /// Publishes the listing to this node's own directory; returns the status line.
  async function publish(page: Page): Promise<string> {
    await ensureListing(page);
    const row = page.locator('.listing-row', { hasText: LISTING_TITLE }).first();
    await row.locator('.publish-directory-did').fill(DIRECTORY_DID!);
    await row.locator('.publish-listing').click();
    const status = row.locator('.publish-status');
    await expect(status).not.toHaveText('', { timeout: 15_000 });
    return status.innerText();
  }

  /// Searches the node's own directory, as the only source, for `cycling`.
  async function searchOwnDirectory(page: Page) {
    await page.getByRole('button', { name: 'Directory', exact: true }).click();
    await expect(page.locator('.directory-sources h3')).toBeVisible({ timeout: 15_000 });
    for (let i = 0; i < 12 && (await page.locator('.directory-source-row').count()) > 0; i++) {
      const n = await page.locator('.directory-source-row').count();
      await page.locator('.directory-source-row').first().locator('.remove-source').click();
      await expect(page.locator('.directory-source-row')).toHaveCount(n - 1, { timeout: 10_000 });
    }
    await page.locator('.add-source-did').fill(DIRECTORY_DID!);
    await page.getByRole('button', { name: 'Add directory' }).click();
    await expect(page.locator('.add-source-status')).toContainText('Added', { timeout: 15_000 });
    await page.locator('.search-categories').fill('cycling');
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    await expect(page.locator('.search-progress')).toContainText('searched', { timeout: 30_000 });
  }

  test('a listing from someone the group does not admit is refused with the reason in words', async ({ page }) => {
    const did = await signIn(page);
    await openSynOrg(page);
    if ((await currentCredential(page, did).count()) === 0) await issueCredential(page, did);
    await revokeCurrentCredential(page, did);

    const status = await publish(page);
    expect(status).toContain('Not published: this group did not admit this listing.');
    expect(status).toContain('Membership revoked');
    expect(status.toLowerCase()).not.toMatch(/\bverified\b/);
  });

  test('the SynOrg issues a credential and the search result shows the membership checked on this node', async ({ page }) => {
    const did = await signIn(page);
    await openSynOrg(page);
    await issueCredential(page, did);
    await expect(page.locator('.no-instant-removal-notice').first()).toHaveText(NO_INSTANT_REMOVAL_NOTICE);
    expect(await publish(page)).toBe('Published.');

    await searchOwnDirectory(page);
    const hit = page.locator('.search-hit', { hasText: LISTING_TITLE });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    const membership = hit.locator('.evidence-membership');
    await expect(membership).toContainText('Member of');
    await expect(membership).toContainText('checked on your node');
    await expect(membership).toContainText('Group withdrawals checked');
  });

  test('suspending the member removes the result, and the held copy shows the suspension only after "check again"', async ({ page }) => {
    const did = await signIn(page);
    await openSynOrg(page);
    await page.locator('.suspend-member-did').fill(did);
    await page.locator('.suspend-rule').fill('r1');
    await page.locator('.suspend-reason').fill('spec: late twice');
    await page.locator('.suspend-member').click();
    await expect(page.locator('.suspend-status')).toHaveText('Suspended.', { timeout: 15_000 });
    await expect(page.locator('.decision-row[data-status="active"]')).toHaveCount(1);

    await searchOwnDirectory(page);
    await expect(page.locator('.search-hit', { hasText: LISTING_TITLE })).toHaveCount(0);

    await page.getByRole('button', { name: 'Memberships', exact: true }).click();
    const row = page.locator('.membership-row', { hasText: did });
    await expect(row).toHaveCount(1, { timeout: 15_000 });
    await expect(page.locator('.no-instant-removal-notice')).toHaveText(NO_INSTANT_REMOVAL_NOTICE);
    // The copy this node already held is not rewritten by a search.
    await expect(row.locator('.membership-words')).toContainText('Member of');

    await row.locator('.check-again').click();
    await expect(row.locator('.membership-words')).toContainText('Membership suspended', { timeout: 15_000 });
    await expect(row.locator('.membership-checked')).toContainText('moments ago');
  });

  test('lifting the suspension brings the result back', async ({ page }) => {
    await signIn(page);
    await openSynOrg(page);
    await page.locator('.decision-row[data-status="active"] .lift-suspension').click();
    await expect(page.locator('.decision-row[data-status="active"]')).toHaveCount(0, { timeout: 15_000 });

    await searchOwnDirectory(page);
    const hit = page.locator('.search-hit', { hasText: LISTING_TITLE });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    await expect(hit.locator('.evidence-membership')).toContainText('Member of');
  });

  test('revoking the credential shows "revoked" on the next check', async ({ page }) => {
    const did = await signIn(page);
    await openSynOrg(page);
    await revokeCurrentCredential(page, did);

    await page.getByRole('button', { name: 'Memberships', exact: true }).click();
    const row = page.locator('.membership-row', { hasText: did });
    await expect(row).toHaveCount(1, { timeout: 15_000 });
    await row.locator('.check-again').click();
    await expect(row.locator('.membership-words')).toContainText('Membership revoked', { timeout: 15_000 });
    await expect(row).toHaveAttribute('data-state', 'revoked');
  });

  test('no membership line ever uses the word "verified"', async ({ page }) => {
    const did = await signIn(page);
    await openSynOrg(page);
    await issueCredential(page, did);
    await searchOwnDirectory(page);
    const hit = page.locator('.search-hit', { hasText: LISTING_TITLE });
    await expect(hit).toHaveCount(1, { timeout: 20_000 });
    const lines = [await hit.locator('.evidence-membership').innerText()];

    await page.getByRole('button', { name: 'Memberships', exact: true }).click();
    const row = page.locator('.membership-row', { hasText: did });
    await expect(row).toHaveCount(1, { timeout: 15_000 });
    lines.push(await row.innerText());

    for (const line of lines) expect(line.toLowerCase(), line).not.toMatch(/\bverified\b/);
  });
});
