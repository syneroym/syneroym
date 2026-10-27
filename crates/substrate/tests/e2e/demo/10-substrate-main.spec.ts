import { expect, test, type Page } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import { caption, newSession, requireEnv, runScript, step } from './steps';

const SCRIPT = '10-substrate-main.sh';
const WS_LAST_UPDATED = 'ws-last-updated';

test.describe.configure({ mode: 'serial' });

test('substrate main story: deploy and call, in the terminal', async () => {
  runScript(SCRIPT);
});

async function openApp(page: Page, appUrl: string) {
  await page.goto(appUrl);
  await page.waitForLoadState('networkidle');
  await expect(page.locator('h1')).toContainText('Hello world from', { timeout: 30_000 });
}

async function openComments(page: Page) {
  await page.click('text=Comments etc.');
  await expect(page.locator('h2')).toContainText('Comments', { timeout: 35_000 });
}

async function postComment(page: Page, text: string) {
  await page.fill('textarea[placeholder="Write a comment..."]', text);
  await page.click('button:has-text("Submit")');
  await expect(page.locator('text=Comment saved!')).toBeVisible({ timeout: 35_000 });
}

test('substrate main story: the web app, live in two sessions', async ({ page, browser }, testInfo) => {
  const env = readDemoEnv();
  const webHost = requireEnv(env, 'WEB_HOST', SCRIPT);
  const appUrl = `http://${webHost}:${env.S1_WEBRTC_BOOT}/?force_tunnel=false`;

  await step(page, 'Session A opens the web app that runs on S1', async () => {
    await openApp(page, appUrl);
  });

  await step(page, 'Session A posts a comment', async () => {
    await openComments(page);
    await expect(page.getByTestId('ws-status')).toHaveText('Connected', { timeout: 35_000 });
    await postComment(page, 'Hello from session A');
  });

  const before = await page.getByTestId(WS_LAST_UPDATED).innerText();
  const second = await newSession(browser, testInfo);
  const other = await second.newPage();
  try {
    await step(other, 'Session B opens the same app', async () => {
      await openApp(other, appUrl);
      await openComments(other);
    });

    await step(other, 'Session B posts a comment', async () => {
      await postComment(other, 'Hello from session B');
    });

    await caption(page, 'Session A sees the change live, without a reload');
    await expect(page.getByTestId(WS_LAST_UPDATED)).not.toHaveText(before, { timeout: 35_000 });
  } finally {
    await second.close();
  }
});
