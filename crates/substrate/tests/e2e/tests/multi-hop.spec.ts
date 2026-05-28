import { test, expect } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

// Scenario 1: Inbound Path (Browser -> C -> Cp -> Sz)
test.describe('WebRTC Multi-Hop Inbound (Browser -> C -> Cp -> Sz)', () => {
  const forceTunnel = true;

  test.beforeEach(async ({ page }) => {
    page.on('console', msg => console.log('BROWSER:', msg.text()));
    const demo1Did = process.env.DEMO1_DID;
    expect(demo1Did).toBeDefined();

    const demo1Alias = process.env.DEMO1_ALIAS;
    expect(demo1Alias).toBeDefined();

    // Access Global Coordinator C's bootstrap page (port 7662)
    const url = `http://${demo1Alias}:7662/?force_tunnel=${forceTunnel}`;
    console.log('Navigating to Inbound Bootstrap URL:', url);

    await page.goto(url);
    await page.waitForLoadState('networkidle');

    // Wait for mock app's header to prove the multi-hop path worked and miniapp is loading
    await expect(page.locator('h1')).toContainText('Hello world from demo1-instance0', { timeout: 35000 });
  });

  test('Inbound GET / and navigate to comments', async ({ page }) => {
    await page.click('text=Comments etc.');
    await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
  });

  test('Inbound POST /api/comments and verify live updates', async ({ page }) => {
    await page.click('text=Comments etc.');
    const commentText = `Inbound E2E comment ${Date.now()}`;
    
    // Fill and submit comment
    await page.fill('textarea[placeholder="Write a comment..."]', commentText);
    await page.click('button:has-text("Submit")');
    
    await expect(page.locator('text=Comment saved!')).toBeVisible({ timeout: 35000 });
    await expect(page.locator('ul').first()).toContainText(commentText, { timeout: 35000 });
  });
});

// Scenario 2: Reverse Path (Browser -> Cp -> C -> Sx)
test.describe('WebRTC Multi-Hop Reverse (Browser -> Cp -> C -> Sx)', () => {
  const forceTunnel = true;

  test.beforeEach(async ({ page }) => {
    page.on('console', msg => console.log('BROWSER:', msg.text()));
    const demo2Did = process.env.DEMO2_DID;
    expect(demo2Did).toBeDefined();

    const demo2Alias = process.env.DEMO2_ALIAS;
    expect(demo2Alias).toBeDefined();

    // Access Private Coordinator Cp's bootstrap page (port 7672)
    const url = `http://${demo2Alias}:7672/?force_tunnel=${forceTunnel}`;
    console.log('Navigating to Reverse Bootstrap URL:', url);

    await page.goto(url);
    await page.waitForLoadState('networkidle');

    // Wait for mock app's header
    await expect(page.locator('h1')).toContainText('Hello world from demo1-instance0', { timeout: 35000 });
  });

  test('Reverse GET / and navigate to comments', async ({ page }) => {
    await page.click('text=Comments etc.');
    await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
  });

  test('Reverse POST /api/comments and verify live updates', async ({ page }) => {
    await page.click('text=Comments etc.');
    const commentText = `Reverse E2E comment ${Date.now()}`;
    
    // Fill and submit comment
    await page.fill('textarea[placeholder="Write a comment..."]', commentText);
    await page.click('button:has-text("Submit")');
    
    await expect(page.locator('text=Comment saved!')).toBeVisible({ timeout: 35000 });
    await expect(page.locator('ul').first()).toContainText(commentText, { timeout: 35000 });
  });
});
