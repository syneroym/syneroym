import { test as base } from '@playwright/test';

export { expect } from '@playwright/test';
export type { Browser, Page } from '@playwright/test';

// The page's console is chatty (every service-worker request, every tunnel
// handshake step): echoing it live made a passing run ~180 KB of output.
// Buffer it instead, and print it only for a test that did not pass, where
// it is the most useful clue.
export const test = base.extend<{ browserConsole: void }>({
  browserConsole: [
    async ({ page }, use, testInfo) => {
      const lines: string[] = [];
      page.on('console', msg => lines.push(`BROWSER: ${msg.text()}`));
      page.on('pageerror', err => lines.push(`PAGE ERROR: ${err.message}`));
      await use();
      if (testInfo.status !== testInfo.expectedStatus && lines.length > 0) {
        console.log(`--- browser console: ${testInfo.title} ---\n${lines.join('\n')}`);
      }
    },
    { auto: true },
  ],
});
