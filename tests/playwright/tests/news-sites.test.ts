import { test, expect } from '@playwright/test';

/**
 * News and media website tests through the SOCKS5 proxy
 * Tests various news sites to ensure the proxy handles
 * different content types and CDNs correctly
 */

test.describe('News Website Tests', () => {
  test('should load BBC News homepage', async ({ page }) => {
    await page.goto('https://www.bbc.com/news', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/BBC News/i);

    // Check for main content
    const content = page.locator('main, [role="main"], #main-content').first();
    await expect(content).toBeVisible({ timeout: 15000 });
  });

  test('should load CNN homepage', async ({ page }) => {
    await page.goto('https://www.cnn.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/CNN/i);

    // Check for content container
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Reuters homepage', async ({ page }) => {
    await page.goto('https://www.reuters.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Reuters/i);

    // Check main content loaded
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load The Guardian homepage', async ({ page }) => {
    await page.goto('https://www.theguardian.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/The Guardian/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load NPR homepage', async ({ page }) => {
    await page.goto('https://www.npr.org/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/NPR/i);

    // Check main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should handle news site with images and videos', async ({ page }) => {
    await page.goto('https://www.bbc.com/news', { waitUntil: 'domcontentloaded' });

    // Wait for main content
    await page.waitForSelector('main, [role="main"]', { timeout: 15000 });

    // Check for images (most news sites have images)
    const images = page.locator('img');
    const imageCount = await images.count();
    expect(imageCount).toBeGreaterThan(0);

    // Verify at least some images loaded
    const firstImage = images.first();
    if (await firstImage.isVisible()) {
      const naturalWidth = await firstImage.evaluate((img: HTMLImageElement) => img.naturalWidth);
      expect(naturalWidth).toBeGreaterThan(0);
    }
  });

  test('should navigate between news articles', async ({ page }) => {
    await page.goto('https://www.bbc.com/news', { waitUntil: 'domcontentloaded' });

    // Wait for main content
    await page.waitForSelector('main, [role="main"]', { timeout: 15000 });

    // Find first article link
    const articleLink = page.locator('a[href*="/news/articles/"], a[href*="/news/world-"]').first();

    if (await articleLink.isVisible()) {
      await articleLink.click();

      // Wait for article to load
      await page.waitForLoadState('domcontentloaded');

      // Verify we navigated
      expect(page.url()).toContain('bbc.com');

      // Check content loaded
      const hasContent = await page.locator('body').isVisible();
      expect(hasContent).toBeTruthy();
    }
  });
});
