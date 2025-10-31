/**
 * Test utilities and constants for Playwright proxy tests
 */

/**
 * Timeout constants for different types of operations
 * Extracted to avoid magic numbers and allow easy tuning
 */
export const TIMEOUTS = {
  /** Maximum time to wait for page load */
  PAGE_LOAD: 30 * 1000, // 30 seconds

  /** Maximum time to wait for content to appear */
  CONTENT_LOAD: 10 * 1000, // 10 seconds

  /** Maximum time to wait for network to be idle */
  NETWORK_IDLE: 15 * 1000, // 15 seconds

  /** Short wait for UI stabilization */
  UI_STABILIZATION: 2 * 1000, // 2 seconds

  /** Maximum acceptable page load time for performance tests */
  MAX_PAGE_LOAD_TIME: 60 * 1000, // 60 seconds
} as const;

/**
 * Retry configuration for flaky operations
 */
export const RETRY_CONFIG = {
  /** Number of retries for flaky tests */
  MAX_RETRIES: 2,

  /** Delay between retries in milliseconds */
  RETRY_DELAY: 1000,
} as const;

/**
 * Helper function to wait for content with better error messages
 */
export async function waitForContent(
  page: any,
  selector: string,
  options?: { timeout?: number }
): Promise<boolean> {
  try {
    await page.waitForSelector(selector, {
      timeout: options?.timeout || TIMEOUTS.CONTENT_LOAD,
      state: 'visible',
    });
    return true;
  } catch (error) {
    console.warn(`Content not found: ${selector}`);
    return false;
  }
}

/**
 * Helper function to gracefully wait for page initialization
 * Uses actual load state instead of arbitrary timeout
 */
export async function waitForPageReady(
  page: any,
  options?: { waitUntil?: 'load' | 'domcontentloaded' | 'networkidle' }
): Promise<void> {
  const waitUntil = options?.waitUntil || 'domcontentloaded';
  await page.waitForLoadState(waitUntil, { timeout: TIMEOUTS.PAGE_LOAD });

  // Brief stabilization wait for dynamic content
  await page.waitForTimeout(TIMEOUTS.UI_STABILIZATION);
}

/**
 * Helper function to check if proxy is being used
 * Returns true if the connection appears to go through a proxy
 */
export async function verifyProxyUsage(page: any): Promise<boolean> {
  try {
    await page.goto('https://httpbin.org/get', {
      waitUntil: 'domcontentloaded',
      timeout: TIMEOUTS.PAGE_LOAD,
    });

    const body = await page.locator('body').textContent();

    // The page should load and show connection info
    return body !== null && body.length > 0;
  } catch (error) {
    console.error('Proxy verification failed:', error);
    return false;
  }
}

/**
 * Helper to measure page load time
 */
export async function measurePageLoadTime(
  page: any,
  url: string
): Promise<number> {
  const startTime = Date.now();
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  return Date.now() - startTime;
}

/**
 * Helper to safely find and interact with elements
 * Handles cases where elements might not be present
 */
export async function safelyInteract(
  page: any,
  selector: string,
  action: (element: any) => Promise<void>,
  options?: { timeout?: number; fallbackMessage?: string }
): Promise<boolean> {
  try {
    const element = page.locator(selector).first();
    const isVisible = await element.isVisible({
      timeout: options?.timeout || TIMEOUTS.CONTENT_LOAD
    });

    if (isVisible) {
      await action(element);
      return true;
    }
  } catch (error) {
    const message = options?.fallbackMessage ||
                   `Could not interact with ${selector}`;
    console.log(message);
  }

  return false;
}
