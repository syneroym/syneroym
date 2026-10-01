import { test, expect, type Page } from '../fixtures';
import { loginWithDelegatedKey, rpcCall } from '../hub-helpers';
import { readE2EPorts } from '../ports';

test.describe.configure({ mode: 'serial' });

const OWNER_CAN_READ_NOTICE =
  "The owner of this group makes and shares the group's key, so the owner can read every message sent while they own it. Adding or removing a member is shown to everyone in the group.";

const GROUP_ADD_UNREACHABLE_MESSAGE =
  "Could not reach this person to add them. Someone you have not talked to before must be online when you add them.";

test.describe('Roym Hub: group chat', () => {
  let HUB_URL: string;

  test.beforeAll(() => {
    HUB_URL = process.env.ROYM_HUB_URL || `http://127.0.0.1:${readE2EPorts().gatewayPort}`;
  });

  async function signIn(page: Page): Promise<string> {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');
    await loginWithDelegatedKey(page);
    const whoami = await rpcCall(page, 'session.whoami', {});
    expect(whoami.result?.did).toBeTruthy();
    return whoami.result.did as string;
  }

  async function openGroups(page: Page) {
    await page.getByRole('button', { name: 'Groups', exact: true }).click();
    await expect(page.locator('.groups-screen')).toBeVisible({ timeout: 15_000 });
  }

  test('creating a group lists it and states that the owner can read it', async ({ page }) => {
    await signIn(page);
    await openGroups(page);

    const nameInput = page.locator('.new-group-box input');
    await nameInput.fill('Street Garden');
    await page.getByRole('button', { name: 'New group', exact: true }).click();

    const groupItem = page.locator('.conversation-items .conversation-item', { hasText: 'Street Garden' });
    await expect(groupItem).toBeVisible({ timeout: 15_000 });
    await groupItem.click();

    await expect(page.locator('.group-title')).toHaveText('Street Garden');
    await expect(page.locator('.group-owner')).toContainText('(you)');
    await expect(page.locator('.owner-can-read-notice')).toHaveText(OWNER_CAN_READ_NOTICE);
  });

  test('a group name with markup renders as literal text', async ({ page }) => {
    await signIn(page);
    await openGroups(page);

    let requestedX = false;
    page.on('request', (req) => {
      if (req.url().includes('/x')) requestedX = true;
    });

    const markup = '<img src=x onerror=alert(1)>';
    const nameInput = page.locator('.new-group-box input');
    await nameInput.fill(markup);
    await page.getByRole('button', { name: 'New group', exact: true }).click();

    const groupItem = page.locator('.conversation-items .conversation-item', { hasText: markup });
    await expect(groupItem).toBeVisible({ timeout: 15_000 });
    await groupItem.click();

    await expect(page.locator('.group-title')).toHaveText(markup);
    expect(await page.locator('.group-title img').count()).toBe(0);
    expect(requestedX).toBe(false);
  });

  test('adding a member who cannot be reached shows why, and changes nothing', async ({ page }) => {
    await signIn(page);
    await openGroups(page);

    const groupItem = page.locator('.conversation-items .conversation-item', { hasText: 'Street Garden' }).first();
    await groupItem.click();

    const unreachableDid = 'did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK';
    const addInput = page.locator('.add-member-box input');
    await addInput.fill(unreachableDid);
    await page.getByRole('button', { name: 'Add member', exact: true }).click();

    const addErr = page.locator('.add-member-box .error-text');
    await expect(addErr).toHaveText(GROUP_ADD_UNREACHABLE_MESSAGE, { timeout: 25_000 });

    const membersHeader = page.locator('.members-box h4');
    await expect(membersHeader).toHaveText('Members (1)');
  });

  test('sending with no other member says there is nobody to deliver to', async ({ page }) => {
    await signIn(page);
    await openGroups(page);

    const groupItem = page.locator('.conversation-items .conversation-item', { hasText: 'Street Garden' }).first();
    await groupItem.click();

    const composerInput = page.locator('.composer-box input');
    await composerInput.fill('message to nobody');
    await page.getByRole('button', { name: 'Send', exact: true }).click();

    const composerErr = page.locator('.composer-box .composer-error');
    await expect(composerErr).toContainText('nowhere to deliver', { timeout: 15_000 });

    const messageRows = page.locator('.message-list .message-row');
    await expect(messageRows).toHaveCount(0);
  });

  test('the group info panel never says verified', async ({ page }) => {
    await signIn(page);
    await openGroups(page);

    const groupItem = page.locator('.conversation-items .conversation-item', { hasText: 'Street Garden' }).first();
    await groupItem.click();

    await expect(page.locator('.key-changed-date')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.transcript-check-code')).toBeVisible({ timeout: 15_000 });

    const keyInfo = await page.locator('.key-changed-date').innerText();
    expect(keyInfo.toLowerCase()).not.toContain('verified');

    const transcriptInfo = await page.locator('.transcript-check-code').innerText();
    expect(transcriptInfo.toLowerCase()).not.toContain('verified');

    const membersInfo = await page.locator('.members-box').innerText();
    expect(membersInfo.toLowerCase()).not.toContain('verified');

    expect(await page.locator('.group-info-panel [data-verified="true"]').count()).toBe(0);
    expect(await page.locator('.message-list [data-verified="true"]').count()).toBe(0);
  });

  test('groups and 1:1 conversations stay in their own tabs', async ({ page }) => {
    await signIn(page);
    await openGroups(page);

    await expect(page.locator('.conversation-items')).toContainText('Street Garden');

    // Switch to Messages tab
    await page.getByRole('button', { name: 'Messages', exact: true }).click();
    await expect(page.locator('.messages-screen')).toBeVisible({ timeout: 15_000 });

    const messagesListText = await page.locator('.conversation-items').innerText();
    expect(messagesListText).not.toContain('Street Garden');

    // Switch back to Groups tab
    await page.getByRole('button', { name: 'Groups', exact: true }).click();
    await expect(page.locator('.conversation-items')).toContainText('Street Garden');
  });
});
