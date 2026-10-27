import { execFileSync } from 'child_process';
import * as path from 'path';
import { test, type Browser, type BrowserContext, type Page, type TestInfo } from '@playwright/test';

// Helpers shared by the demo story specs.

const PACE_MS = Number(process.env.DEMO_PACE_MS ?? 600);
const present = process.env.DEMO_MODE === 'present';

/**
 * Run a shell story script. Skipped with DEMO_ATTACH=1: the presenter has
 * already run the scripts by hand, and running one twice would fail or deploy
 * twice.
 */
export function runScript(name: string): void {
  if (process.env.DEMO_ATTACH === '1') {
    console.log(`DEMO_ATTACH=1: not running ${name}.`);
    return;
  }
  execFileSync('bash', [path.join(__dirname, name)], { stdio: 'inherit' });
}

/** A value a script wrote to the env file. Stops with the script's name when it is missing. */
export function requireEnv(env: Record<string, string>, key: string, script: string): string {
  const value = env[key];
  if (!value) throw new Error(`${key} is not in the demo env file. Run ${script} first.`);
  return value;
}

const lastCaption = new WeakMap<Page, string>();

async function paintCaption(page: Page, text: string) {
  await page.evaluate((caption) => {
    let bar = document.getElementById('demo-caption');
    if (!bar) {
      bar = document.createElement('div');
      bar.id = 'demo-caption';
      // Set through the CSSOM: a page with a strict style CSP rejects a style attribute.
      bar.style.cssText =
        'position:fixed;left:0;right:0;bottom:0;z-index:2147483647;padding:10px 16px;' +
        'background:#1d2433;color:#fff;font:600 18px system-ui,sans-serif;text-align:center;' +
        'pointer-events:none';
      document.documentElement.appendChild(bar);
    }
    bar.textContent = caption;
  }, text);
}

/**
 * Show a caption in the page and print it. A new page load wipes the bar, so
 * it is painted again after every navigation.
 */
export async function caption(page: Page, text: string): Promise<void> {
  console.log(`\n== ${text} ==`);
  if (!lastCaption.has(page)) {
    page.on('domcontentloaded', () => {
      const current = lastCaption.get(page);
      if (current) paintCaption(page, current).catch(() => undefined);
    });
  }
  lastCaption.set(page, text);
  await paintCaption(page, text);
  if (present) await page.waitForTimeout(PACE_MS);
}

/** One named story step: a caption, then the work, shown as a step in the report. */
export async function step(page: Page, title: string, body: () => Promise<void>): Promise<void> {
  await test.step(title, async () => {
    await caption(page, title);
    await body();
  });
}

/**
 * A second, separate browser session. Playwright records video only for the
 * built-in `page`, so a context made here has to ask for it.
 */
export function newSession(browser: Browser, testInfo: TestInfo): Promise<BrowserContext> {
  const recordVideo = process.env.DEMO_VIDEO === '1' ? { dir: testInfo.outputDir } : undefined;
  return browser.newContext({ recordVideo });
}
