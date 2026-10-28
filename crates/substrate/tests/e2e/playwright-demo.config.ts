import { defineConfig, devices } from '@playwright/test';

// DEMO_MODE=present: headed browser, slow motion, a pause after each step.
// DEMO_VIDEO=1:      record every browser page to demo-output/.
const present = process.env.DEMO_MODE === 'present';
const video = process.env.DEMO_VIDEO === '1';

export default defineConfig({
  testDir: './demo',
  testMatch: '**/*.spec.ts',
  outputDir: './demo-output',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1, // The stories are cumulative and share two nodes.
  // Two nodes boot and Roym deploys twice, so this is longer than the main suite's.
  globalTimeout: 900_000,
  timeout: 180_000,
  reporter: [['list']],
  globalSetup: require.resolve('./demo-global-setup.ts'),
  globalTeardown: require.resolve('./demo-global-teardown.ts'),
  use: {
    headless: !present,
    video: video ? 'on' : 'off',
    trace: video ? 'on' : 'off',
    screenshot: 'off',
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          slowMo: present ? 250 : 0,
          args: [
            '--allow-loopback-in-peer-connection',
            '--disable-features=WebRtcHideLocalIpsWithMdns',
          ],
        },
      },
    },
  ],
});
