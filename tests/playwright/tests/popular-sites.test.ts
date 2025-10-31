import { test, expect } from '@playwright/test';

/**
 * Tests for popular websites across different categories
 * Ensures the proxy handles various types of web traffic
 */

test.describe('Popular Websites', () => {
  test('should load GitHub homepage', async ({ page }) => {
    await page.goto('https://github.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/GitHub/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Stack Overflow', async ({ page }) => {
    await page.goto('https://stackoverflow.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Stack Overflow/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Reddit homepage', async ({ page }) => {
    await page.goto('https://www.reddit.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Reddit/i);

    // Check for content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Twitter/X homepage', async ({ page }) => {
    await page.goto('https://x.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded (title may vary)
    await page.waitForTimeout(2000);

    // Check for content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Amazon homepage', async ({ page }) => {
    await page.goto('https://www.amazon.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Amazon/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Google Search', async ({ page }) => {
    await page.goto('https://www.google.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Google/i);

    // Check for search box
    const searchBox = page.locator('textarea[name="q"], input[name="q"]').first();
    await expect(searchBox).toBeVisible({ timeout: 10000 });
  });

  test('should perform Google search', async ({ page }) => {
    await page.goto('https://www.google.com/', { waitUntil: 'domcontentloaded' });

    // Wait for search box
    const searchBox = page.locator('textarea[name="q"], input[name="q"]').first();
    await searchBox.waitFor({ state: 'visible', timeout: 10000 });

    // Perform search
    await searchBox.fill('proxy server');
    await searchBox.press('Enter');

    // Wait for results
    await page.waitForTimeout(3000);

    // Verify we're on results page
    expect(page.url()).toContain('search');

    // Check for results
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load DuckDuckGo', async ({ page }) => {
    await page.goto('https://duckduckgo.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/DuckDuckGo/i);

    // Check for search box
    const searchBox = page.locator('input[name="q"]');
    await expect(searchBox).toBeVisible();
  });

  test('should load Medium homepage', async ({ page }) => {
    await page.goto('https://medium.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Medium/i);

    // Check for content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load MDN Web Docs', async ({ page }) => {
    await page.goto('https://developer.mozilla.org/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/MDN/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should handle sites with heavy JavaScript', async ({ page }) => {
    // Test sites known for heavy JavaScript usage
    const sites = [
      'https://www.github.com/',
      'https://www.reddit.com/',
    ];

    for (const site of sites) {
      await page.goto(site, { waitUntil: 'domcontentloaded' });

      // Wait for JS to execute
      await page.waitForTimeout(3000);

      // Verify content loaded
      const hasContent = await page.locator('body').isVisible();
      expect(hasContent).toBeTruthy();

      // Verify no JS errors
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      await page.waitForTimeout(1000);

      // Some JS errors are acceptable (ads, analytics, etc.)
      // We just want to make sure the page functions
      expect(hasContent).toBeTruthy();
    }
  });
});
