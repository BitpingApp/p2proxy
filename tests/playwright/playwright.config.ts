import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright configuration for P2Proxy SOCKS5 testing
 *
 * This configuration sets up Playwright to route all traffic through
 * the local P2Proxy SOCKS5 server running on localhost:1080
 */
export default defineConfig({
  testDir: './tests',

  // Test timeout settings
  timeout: 120 * 1000, // 120 seconds per test (increased for P2P proxy)
  expect: {
    timeout: 30 * 1000, // 30 seconds for assertions (increased for P2P proxy)
  },

  // Run tests in parallel
  fullyParallel: true,

  // Fail the build on CI if you accidentally left test.only in the source code
  forbidOnly: !!process.env.CI,

  // Retry on CI only
  retries: process.env.CI ? 2 : 0,

  // Limit workers on CI for stability
  workers: process.env.CI ? 2 : undefined,

  // Reporter configuration
  reporter: [
    ['html', { outputFolder: 'playwright-report' }],
    ['json', { outputFile: 'test-results/results.json' }],
    ['list'],
  ],

  // Global test configuration
  use: {
    // Base URL for relative navigation
    baseURL: 'https://www.wikipedia.org',

    // Collect trace on first retry
    trace: 'on-first-retry',

    // Screenshot on failure
    screenshot: 'only-on-failure',

    // Video on first retry
    video: 'retain-on-failure',

    // Navigation timeout
    navigationTimeout: 90 * 1000, // 90 seconds (increased for P2P proxy)

    // Action timeout
    actionTimeout: 30 * 1000, // 30 seconds (increased for P2P proxy)

    // Ignore HTTPS errors (required for SOCKS5 proxy)
    ignoreHTTPSErrors: true,

    // SOCKS5 Proxy configuration - route all traffic through P2Proxy
    proxy: {
      server: 'socks5://localhost:1080',
    },
  },

  // Projects for different browsers
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        // Override proxy settings if needed per project
        proxy: {
          server: 'socks5://localhost:1080',
        },
        // Anti-bot detection settings
        launchOptions: {
          args: [
            '--disable-blink-features=AutomationControlled',
            '--disable-features=IsolateOrigins,site-per-process',
            '--disable-site-isolation-trials',
          ],
        },
        // Realistic user agent and viewport
        userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
        viewport: { width: 1920, height: 1080 },
        hasTouch: false,
        isMobile: false,
      },
    },

    // Uncomment to test with other browsers
    // Note: SOCKS5 proxy support may vary
    // {
    //   name: 'firefox',
    //   use: {
    //     ...devices['Desktop Firefox'],
    //     proxy: {
    //       server: 'socks5://localhost:1080',
    //     },
    //   },
    // },

    // {
    //   name: 'webkit',
    //   use: {
    //     ...devices['Desktop Safari'],
    //     proxy: {
    //       server: 'socks5://localhost:1080',
    //     },
    //   },
    // },
  ],

  // Web server configuration (not needed for proxy tests)
  // webServer: undefined,
});
