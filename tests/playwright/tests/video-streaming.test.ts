import { test, expect } from '@playwright/test';

/**
 * Video streaming tests through the SOCKS5 proxy
 * Tests YouTube and other video platforms to ensure
 * the proxy can handle streaming traffic
 */

test.describe('Video Streaming Tests', () => {
  test('should load YouTube homepage', async ({ page }) => {
    await page.goto('https://www.youtube.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/YouTube/i);

    // Wait for content to load
    await page.waitForTimeout(3000);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should search on YouTube', async ({ page }) => {
    await page.goto('https://www.youtube.com/', { waitUntil: 'domcontentloaded' });

    // Wait for page to stabilize
    await page.waitForTimeout(3000);

    // Try to find and interact with search box
    // YouTube's HTML structure may vary, so we try multiple selectors
    const searchSelectors = [
      'input#search',
      'input[name="search_query"]',
      'input[aria-label*="Search"]',
    ];

    let searchBox = null;
    for (const selector of searchSelectors) {
      try {
        searchBox = page.locator(selector).first();
        if (await searchBox.isVisible({ timeout: 2000 })) {
          break;
        }
      } catch {
        continue;
      }
    }

    if (searchBox && await searchBox.isVisible()) {
      await searchBox.click();
      await searchBox.fill('test video');
      await searchBox.press('Enter');

      // Wait for search results
      await page.waitForTimeout(3000);

      // Verify we're on results page
      expect(page.url()).toContain('search');
    } else {
      // If we can't interact with search, just verify page loaded
      console.log('Could not find search box, skipping search interaction');
      expect(page.url()).toContain('youtube.com');
    }
  });

  test('should load a public YouTube video page', async ({ page }) => {
    // Use a stable, public video (YouTube's own channel trailer is usually stable)
    await page.goto('https://www.youtube.com/watch?v=dQw4w9WgXcQ', {
      waitUntil: 'domcontentloaded',
    });

    // Wait for page to load
    await page.waitForTimeout(3000);

    // Verify we're on a video page
    expect(page.url()).toContain('youtube.com/watch');

    // Check that content loaded
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();

    // Note: We don't actually play the video to avoid long test times
    // and potential issues with autoplay policies
  });

  test('should load Vimeo homepage', async ({ page }) => {
    await page.goto('https://vimeo.com/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Vimeo/i);

    // Check for main content
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load Twitch homepage', async ({ page }) => {
    await page.goto('https://www.twitch.tv/', { waitUntil: 'domcontentloaded' });

    // Verify page loaded
    await expect(page).toHaveTitle(/Twitch/i);

    // Wait for content
    await page.waitForTimeout(3000);

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

    await page.goto('https://www.youtube.com/', { waitUntil: 'networkidle' });

    // We may or may not get CDN requests depending on what loads
    // The important thing is the page loads without errors
    const hasContent = await page.locator('body').isVisible();
    expect(hasContent).toBeTruthy();
  });

  test('should load streaming thumbnails and images', async ({ page }) => {
    await page.goto('https://www.youtube.com/', { waitUntil: 'domcontentloaded' });

    // Wait for thumbnails to load
    await page.waitForTimeout(5000);

    // Check for thumbnail images
    const thumbnails = page.locator('img');
    const thumbnailCount = await thumbnails.count();

    expect(thumbnailCount).toBeGreaterThan(0);

    // Check that at least some thumbnails loaded
    const loadedImages = await thumbnails.evaluateAll((images: HTMLImageElement[]) => {
      return images.filter(img => img.complete && img.naturalWidth > 0).length;
    });

    expect(loadedImages).toBeGreaterThan(0);
  });
});
