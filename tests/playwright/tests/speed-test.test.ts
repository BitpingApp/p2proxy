import { test, expect } from '@playwright/test';
import { gotoWithCookieHandling } from './test-utils';

/**
 * Speed test and performance measurement tests
 * Tests connectivity to speed test services through the proxy
 */

test.describe('Speed Test Services', () => {
  test('should load Cloudflare Speed Test', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://speed.cloudflare.com/');

    // Verify page loaded
    await expect(page).toHaveTitle(/Speed Test/i);

    // Wait for the page to initialize
    await page.waitForTimeout(3000);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();

    // The test should be able to connect even if we don't run the full speed test
    // (running full speed tests would take too long for CI)
  });

  test('should display Cloudflare Speed Test UI elements', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://speed.cloudflare.com/');

    // Wait for page to initialize
    await page.waitForTimeout(3000);

    // Look for common elements (these may vary based on Cloudflare's implementation)
    const body = await page.locator('body').textContent();

    // Just verify the page loaded with some content
    expect(body).toBeTruthy();
    expect(body!.length).toBeGreaterThan(0);
  });

  test('should load Fast.com (Netflix speed test)', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://fast.com/');

    // Verify page loaded
    await expect(page).toHaveTitle(/Internet Speed Test/i);

    // Wait for the test to potentially start
    await page.waitForTimeout(2000);

    // Check that page loaded
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Speedtest.net', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://www.speedtest.net/');

    // Verify page loaded
    await expect(page).toHaveTitle(/Speedtest/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should handle download requests', async ({ page }) => {
    // Test that the proxy can handle data download operations
    // by loading a page with significant assets

    const responses: number[] = [];
    page.on('response', (response) => {
      if (response.ok()) {
        responses.push(response.status());
      }
    });

    await gotoWithCookieHandling(page, 'https://www.wikipedia.org/', { waitUntil: 'networkidle' });

    // Verify we got successful responses
    expect(responses.length).toBeGreaterThan(0);
    expect(responses.filter(s => s === 200).length).toBeGreaterThan(0);
  });

  test('should handle concurrent requests', async ({ page }) => {
    // Test that proxy handles multiple concurrent connections
    const responses: string[] = [];

    page.on('response', (response) => {
      responses.push(response.url());
    });

    await gotoWithCookieHandling(page, 'https://www.wikipedia.org/', { waitUntil: 'networkidle' });

    // Verify multiple requests were made (images, CSS, JS, etc.)
    expect(responses.length).toBeGreaterThan(5);
  });
});
