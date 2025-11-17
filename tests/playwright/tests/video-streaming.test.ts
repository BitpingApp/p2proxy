import { test, expect } from '@playwright/test';
import { waitForPageReady, safelyInteract, TIMEOUTS, gotoWithCookieHandling } from './test-utils';

/**
 * Video streaming tests through the SOCKS5 proxy
 * Tests YouTube and other video platforms to ensure
 * the proxy can handle streaming traffic
 */

test.describe.skip('Video Streaming Tests', () => {
  // ALL VIDEO STREAMING TESTS SKIPPED: YouTube/Vimeo/Twitch have bot detection
  // Configure retries for this test suite due to external service variability
  test.describe.configure({ retries: 2 });

  test('should load YouTube homepage', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://www.youtube.com/');

    // Verify page loaded
    await expect(page).toHaveTitle(/YouTube/i);

    // Wait for content to load dynamically
    await waitForPageReady(page);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should search on YouTube', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://www.youtube.com/');

    // Wait for page to stabilize
    await waitForPageReady(page);

    // Try to find and interact with search box
    // YouTube's HTML structure may vary, so we try multiple selectors
    const searchSelectors = [
      'input#search',
      'input[name="search_query"]',
      'input[aria-label*="Search"]',
    ];

    let searchInteracted = false;
    for (const selector of searchSelectors) {
      searchInteracted = await safelyInteract(
        page,
        selector,
        async (searchBox) => {
          await searchBox.click();
          await searchBox.fill('test video');
          await searchBox.press('Enter');

          // Wait for navigation to search results
          await page.waitForLoadState('domcontentloaded', {
            timeout: TIMEOUTS.PAGE_LOAD,
          });
        },
        {
          timeout: TIMEOUTS.CONTENT_LOAD,
          fallbackMessage: `Could not find search box with selector: ${selector}`,
        }
      );

      if (searchInteracted) break;
    }

    if (searchInteracted) {
      // Verify we're on results page
      expect(page.url()).toContain('search');
    } else {
      // If we can't interact with search, just verify page loaded
      console.log('Could not find any search box, skipping search interaction');
      expect(page.url()).toContain('youtube.com');
    }
  });

  test('should load a public YouTube video page', async ({ page }) => {
    // Use a stable, public video (YouTube's own channel trailer is usually stable)
    await gotoWithCookieHandling(page, 'https://www.youtube.com/watch?v=dQw4w9WgXcQ');

    // Wait for page to load dynamically
    await waitForPageReady(page);

    // Verify we're on a video page
    expect(page.url()).toContain('youtube.com/watch');

    // Check that content loaded
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();

    // Note: We don't actually play the video to avoid long test times
    // and potential issues with autoplay policies
  });

  test('should load Vimeo homepage', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://vimeo.com/');

    // Verify page loaded
    await expect(page).toHaveTitle(/Vimeo/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Twitch homepage', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://www.twitch.tv/');

    // Verify page loaded
    await expect(page).toHaveTitle(/Twitch/i);

    // Wait for dynamic content to load
    await waitForPageReady(page);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should handle video CDN requests', async ({ page }) => {
    const cdnResponses: string[] = [];

    page.on('response', (response) => {
      const url = response.url();
      // Look for common video CDN patterns
      if (
        url.includes('googlevideo.com') ||
        url.includes('cloudflare') ||
        url.includes('cdn') ||
        url.includes('.m3u8') ||
        url.includes('.ts')
      ) {
        cdnResponses.push(url);
      }
    });

    await gotoWithCookieHandling(page, 'https://www.youtube.com/', {
      waitUntil: 'networkidle',
    });

    // Verify page loads without errors
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();

    // Log CDN requests found (optional - may be 0 if no video autoplay)
    if (cdnResponses.length > 0) {
      console.log(`Found ${cdnResponses.length} CDN requests`);
    }
  });

  test('should load streaming thumbnails and images', async ({ page }) => {
    await gotoWithCookieHandling(page, 'https://www.youtube.com/');

    // Wait for thumbnails to load dynamically
    await page.waitForLoadState('networkidle', { timeout: TIMEOUTS.NETWORK_IDLE });

    // Check for thumbnail images
    const thumbnails = page.locator('img');
    const thumbnailCount = await thumbnails.count();

    expect(thumbnailCount).toBeGreaterThan(0);

    // Check that at least some thumbnails loaded
    const loadedImages = await thumbnails.evaluateAll((images: HTMLImageElement[]) => {
      return images.filter((img) => img.complete && img.naturalWidth > 0).length;
    });

    expect(loadedImages).toBeGreaterThan(0);
  });
});
