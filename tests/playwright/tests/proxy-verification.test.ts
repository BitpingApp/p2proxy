import { test, expect } from '@playwright/test';
import { verifyProxyUsage, TIMEOUTS, gotoWithCookieHandling } from './test-utils';

/**
 * Proxy verification tests
 * These tests ensure that traffic is actually routed through the SOCKS5 proxy
 * and that the proxy configuration is working correctly
 */

test.describe('Proxy Verification', () => {
  test('should route traffic through SOCKS5 proxy', async ({ page }) => {
    // Verify proxy is actually being used
    const result = await verifyProxyUsage(page);
    if (!result.success) {
      console.error(`Proxy verification failed: ${result.error} - ${result.details}`);
    }
    expect(result.success).toBeTruthy();
  });

  test('should successfully connect through proxy', async ({ page }) => {
    // Use httpbin.org to verify connection details
    await gotoWithCookieHandling(page, 'https://httpbin.org/get');

    // Verify we got a response
    const body = await page.locator('body').textContent();
    expect(body).toBeTruthy();
    expect(body).toContain('httpbin.org');

    // Should show connection was successful
    const content = await page.locator('pre, body').textContent();
    expect(content).toBeTruthy();
  });

  test('should handle HTTPS through proxy', async ({ page }) => {
    // Verify HTTPS works through SOCKS5
    await gotoWithCookieHandling(page, 'https://httpbin.org/status/200');

    // Should successfully load
    const body = await page.locator('body').isVisible();
    expect(body).toBeTruthy();
  });

  test('should expose proxy metrics endpoint', async ({ request }) => {
    // Test that Prometheus metrics are exposed
    const response = await request.get('http://localhost:9091/metrics', {
      timeout: TIMEOUTS.CONTENT_LOAD,
    });

    expect(response.ok()).toBeTruthy();

    const metricsText = await response.text();
    expect(metricsText).toContain('p2proxy');
  });

  test('should fail with non-existent proxy', async ({ browser }) => {
    // Negative test: verify that without a working proxy, connections fail
    const context = await browser.newContext({
      proxy: {
        server: 'socks5://localhost:9999', // Non-existent proxy port
      },
    });

    try {
      const page = await context.newPage();

      // Should fail to connect
      let connectionFailed = false;
      try {
        await page.goto('https://www.wikipedia.org/', {
          timeout: 5000, // Short timeout
        });
      } catch (error) {
        connectionFailed = true;
      }

      expect(connectionFailed).toBeTruthy();
    } finally {
      // Ensure context is always closed
      await context.close();
    }
  });

  test('should preserve headers through proxy', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://httpbin.org/headers');

    const body = await page.locator('body').textContent();
    expect(body).toBeTruthy();

    // Should contain headers information
    expect(body).toContain('headers');
  });

  test('should handle cookies through proxy', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://httpbin.org/cookies/set?test=proxy_value');

    // Get cookies
    const cookies = await page.context().cookies();

    // Should have received the cookie
    expect(cookies.length).toBeGreaterThan(0);

    // Verify cookie was set
    const testCookie = cookies.find((c) => c.name === 'test');
    expect(testCookie).toBeDefined();
    expect(testCookie?.value).toBe('proxy_value');
  });

  test('should handle multiple concurrent proxy connections', async ({ browser }) => {
    const context = await browser.newContext({
      proxy: {
        server: 'socks5://localhost:1080',
      },
    });

    try {
      // Create multiple pages
      const pages = await Promise.all([
        context.newPage(),
        context.newPage(),
        context.newPage(),
      ]);

      // Navigate concurrently to different endpoints
      const navigations = pages.map((page, index) =>
        gotoWithCookieHandling(page, `https://httpbin.org/delay/${index}`)
      );

      // All should complete successfully
      await Promise.all(navigations);

      // Verify all pages loaded
      for (const page of pages) {
        const hasContent = await page.locator('body').isVisible();
        expect(hasContent).toBeTruthy();
        await page.close();
      }
    } finally {
      // Ensure context is always closed
      await context.close();
    }
  });
});
