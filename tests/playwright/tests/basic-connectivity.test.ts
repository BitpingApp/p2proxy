import { test, expect } from '@playwright/test';

/**
 * Basic connectivity tests through the SOCKS5 proxy
 * These tests verify that the proxy can establish connections
 * and route traffic correctly
 */

test.describe('Basic Proxy Connectivity', () => {
  test('should connect to Wikipedia homepage', async ({ page }) => {
    await page.goto('https://www.wikipedia.org/');

    // Verify page loaded successfully
    await expect(page).toHaveTitle(/Wikipedia/);

    // Check for the Wikipedia logo
    const logo = page.locator('.central-featured-logo');
    await expect(logo).toBeVisible();

    // Check for search input
    const searchInput = page.locator('#searchInput');
    await expect(searchInput).toBeVisible();
  });

  test('should navigate to English Wikipedia', async ({ page }) => {
    await page.goto('https://www.wikipedia.org/');

    // Click English Wikipedia
    await page.click('#js-link-box-en');

    // Wait for navigation
    await page.waitForURL(/en.wikipedia.org/);

    // Verify we're on English Wikipedia
    await expect(page).toHaveTitle(/Wikipedia/);

    // Verify main page content loaded
    const mainPage = page.locator('#mp-welcometo');
    await expect(mainPage).toBeVisible();
  });

  test('should perform Wikipedia search', async ({ page }) => {
    await page.goto('https://en.wikipedia.org/');

    // Search for "Proxy server"
    await page.fill('#searchInput', 'Proxy server');
    await page.press('#searchInput', 'Enter');

    // Wait for search results or article
    await page.waitForURL(/.*wiki.*/);

    // Verify content loaded
    const content = page.locator('#content');
    await expect(content).toBeVisible();

    // Check that we have article content or search results
    const hasArticle = await page.locator('#mw-content-text').isVisible();
    expect(hasArticle).toBeTruthy();
  });

  test('should load Wikipedia article with images', async ({ page }) => {
    await page.goto('https://en.wikipedia.org/wiki/Internet');

    // Verify article title
    await expect(page.locator('#firstHeading')).toContainText('Internet');

    // Verify article content loaded
    const content = page.locator('#mw-content-text');
    await expect(content).toBeVisible();

    // Check that images loaded (if any)
    const images = page.locator('img.mw-file-element');
    const imageCount = await images.count();
    if (imageCount > 0) {
      // Verify at least one image is visible
      await expect(images.first()).toBeVisible();
    }
  });

  test('should handle HTTPS connections', async ({ page }) => {
    // Test multiple HTTPS sites to ensure SSL works through proxy
    const sites = [
      'https://www.wikipedia.org/',
      'https://en.wikipedia.org/',
    ];

    for (const site of sites) {
      await page.goto(site);
      await expect(page).not.toHaveTitle('');

      // Verify no security errors
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));

      // Wait a bit to catch any errors
      await page.waitForTimeout(1000);

      expect(errors).toEqual([]);
    }
  });
});
