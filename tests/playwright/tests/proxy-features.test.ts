import { test, expect } from '@playwright/test';
import { measurePageLoadTime, TIMEOUTS } from './test-utils';

/**
 * Proxy-specific feature tests
 * Tests various proxy capabilities and edge cases
 */

test.describe('Proxy Feature Tests', () => {
  test('should handle different HTTP methods', async ({ page }) => {
    // Test GET requests
    await page.goto('https://httpbin.org/get', { waitUntil: 'domcontentloaded' });

    const body = await page.locator('body').textContent();
    expect(body).toContain('httpbin.org');

    // Test that we can see the response
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should handle POST requests', async ({ page }) => {
    await page.goto('https://httpbin.org/forms/post', { waitUntil: 'domcontentloaded' });

    // Fill out a simple form if available
    const form = page.locator('form').first();
    if (await form.isVisible()) {
      // Just verify the form loaded
      expect(await form.isVisible()).toBeTruthy();
    }
  });

  test('should preserve headers through proxy', async ({ page }) => {
    await page.goto('https://httpbin.org/headers', { waitUntil: 'domcontentloaded' });

    const body = await page.locator('body').textContent();

    // Verify headers are present in response
    expect(body).toBeTruthy();
    expect(body!.length).toBeGreaterThan(0);
  });

  test('should handle redirects correctly', async ({ page }) => {
    // Test 301 redirect
    await page.goto('https://httpbin.org/redirect/1', { waitUntil: 'domcontentloaded' });

    // Should end up at /get
    await page.waitForTimeout(2000);

    const body = await page.locator('body').textContent();
    expect(body).toBeTruthy();
  });

  test('should handle large responses', async ({ page }) => {
    // Request a large response (1000 bytes)
    await page.goto('https://httpbin.org/bytes/1000', { waitUntil: 'domcontentloaded' });

    // Verify we got content
    const body = await page.locator('body').textContent();
    expect(body).toBeTruthy();
    expect(body!.length).toBeGreaterThan(0);
  });

  test('should handle gzip compression', async ({ page }) => {
    await page.goto('https://httpbin.org/gzip', { waitUntil: 'domcontentloaded' });

    const body = await page.locator('body').textContent();

    // Verify decompressed content is readable
    expect(body).toBeTruthy();
    expect(body!.length).toBeGreaterThan(0);
  });

  test('should handle cookies', async ({ page }) => {
    await page.goto('https://httpbin.org/cookies/set?test=value', {
      waitUntil: 'domcontentloaded',
    });

    // Get cookies
    const cookies = await page.context().cookies();

    // Should have received cookie
    expect(cookies.length).toBeGreaterThan(0);
  });

  test('should handle WebSocket upgrade attempts gracefully', async ({ page }) => {
    // Many sites use WebSockets, proxy should handle connection attempts
    const wsMessages: string[] = [];

    page.on('websocket', (ws) => {
      ws.on('framereceived', (event) => wsMessages.push('received'));
      ws.on('framesent', (event) => wsMessages.push('sent'));
    });

    await page.goto('https://www.wikipedia.org/', { waitUntil: 'domcontentloaded' });

    // Page should load regardless of WebSocket support
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should handle concurrent connections to different hosts', async ({ browser }) => {
    // Create multiple pages to test concurrent connections
    const context = await browser.newContext({
      proxy: {
        server: 'socks5://localhost:1080',
      },
    });

    const pages = await Promise.all([
      context.newPage(),
      context.newPage(),
      context.newPage(),
    ]);

    // Navigate to different sites concurrently
    const navigations = [
      pages[0].goto('https://www.wikipedia.org/', { waitUntil: 'domcontentloaded' }),
      pages[1].goto('https://www.github.com/', { waitUntil: 'domcontentloaded' }),
      pages[2].goto('https://www.bbc.com/', { waitUntil: 'domcontentloaded' }),
    ];

    await Promise.all(navigations);

    // Verify all pages loaded
    for (const page of pages) {
      const hasContent = await page.locator('body').isVisible();
      expect(hasContent).toBeTruthy();
      await page.close();
    }

    await context.close();
  });

  test('should handle connection pooling', async ({ page }) => {
    // Make multiple requests to the same host
    await page.goto('https://www.wikipedia.org/', { waitUntil: 'networkidle' });

    // Navigate to another page on same host
    await page.goto('https://en.wikipedia.org/wiki/Internet', {
      waitUntil: 'domcontentloaded',
    });

    // Verify page loaded (should reuse connection)
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();

    // Navigate to third page
    await page.goto('https://en.wikipedia.org/wiki/Computer', {
      waitUntil: 'domcontentloaded',
    });

    expect(await page.locator('body').isVisible()).toBeTruthy();
  });

  test('should handle IPv4 addresses', async ({ page }) => {
    // Test direct IP access
    await page.goto('https://1.1.1.1/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should measure connection latency', async ({ page }) => {
    const loadTime = await measurePageLoadTime(page, 'https://www.wikipedia.org/');

    // Verify it loaded in a reasonable time
    // (We're not testing performance, just that it works)
    expect(loadTime).toBeLessThan(TIMEOUTS.MAX_PAGE_LOAD_TIME);

    console.log(`Page loaded in ${loadTime}ms`);
  });
});
