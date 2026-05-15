# P2Proxy Playwright Tests

This directory contains end-to-end (E2E) tests for P2Proxy using Playwright. These tests verify that the SOCKS5 proxy correctly routes traffic to various websites and services.

## Overview

The test suite validates P2Proxy functionality by:

- Testing connectivity to popular websites (Wikipedia, GitHub, news sites, etc.)
- Verifying video streaming platforms (YouTube, Vimeo, Twitch)
- Testing speed test services (Cloudflare Speed Test, Fast.com)
- Validating proxy-specific features (headers, redirects, compression, etc.)
- Ensuring concurrent connections work correctly

## Prerequisites

- Node.js 20 or higher
- P2Proxy built and ready to run (`cargo build --release --bin p2proxy`)
- Valid `BITPING_API_KEY` for authentication

## Installation

```bash
cd tests/playwright
npm install
npx playwright install chromium
```

## Running Tests

### Start P2Proxy

First, start the P2Proxy daemon with your API key:

```bash
# From repository root
BITPING_API_KEY=your_api_key_here ./target/release/p2proxy
```

The proxy will listen on `localhost:1080` (SOCKS5) by default.

### Run All Tests

```bash
cd tests/playwright
npm test
```

### Run Specific Test Files

```bash
# Basic connectivity tests
npx playwright test basic-connectivity.test.ts

# News site tests
npx playwright test news-sites.test.ts

# Video streaming tests
npx playwright test video-streaming.test.ts

# Speed test services
npx playwright test speed-test.test.ts

# Popular websites
npx playwright test popular-sites.test.ts

# Proxy features
npx playwright test proxy-features.test.ts
```

### Run Tests in UI Mode

```bash
npm run test:ui
```

### Debug Tests

```bash
npm run test:debug
```

### Run Tests in Headed Mode

```bash
npm run test:headed
```

## Test Structure

```
tests/playwright/
├── package.json              # npm dependencies
├── playwright.config.ts      # Playwright configuration with SOCKS5 proxy setup
├── tests/
│   ├── basic-connectivity.test.ts    # Basic proxy connectivity tests
│   ├── news-sites.test.ts            # News website tests (BBC, CNN, Reuters, etc.)
│   ├── video-streaming.test.ts       # Video platform tests (YouTube, Vimeo, etc.)
│   ├── speed-test.test.ts            # Speed test service tests
│   ├── popular-sites.test.ts         # Popular website tests (GitHub, Google, etc.)
│   └── proxy-features.test.ts        # Proxy-specific feature tests
└── README.md                 # This file
```

## Test Categories

### Basic Connectivity (`basic-connectivity.test.ts`)

Tests fundamental proxy functionality:
- Loading Wikipedia homepage
- Navigating between pages
- Performing searches
- Loading images and media
- HTTPS connection handling

### News Sites (`news-sites.test.ts`)

Tests news and media websites:
- BBC News
- CNN
- Reuters
- The Guardian
- NPR
- Image and video content loading
- Article navigation

### Video Streaming (`video-streaming.test.ts`)

Tests video streaming platforms:
- YouTube (homepage, search, video pages)
- Vimeo
- Twitch
- CDN request handling
- Thumbnail loading

### Speed Test Services (`speed-test.test.ts`)

Tests connectivity to speed test services:
- Cloudflare Speed Test
- Fast.com (Netflix)
- Speedtest.net
- Download request handling
- Concurrent request handling

### Popular Sites (`popular-sites.test.ts`)

Tests various popular websites:
- GitHub
- Stack Overflow
- Reddit
- Twitter/X
- Amazon
- Google Search
- DuckDuckGo
- Medium
- MDN Web Docs
- Heavy JavaScript sites

### Proxy Features (`proxy-features.test.ts`)

Tests proxy-specific capabilities:
- Different HTTP methods (GET, POST)
- Header preservation
- Redirects (301, 302)
- Large responses
- Gzip compression
- Cookie handling
- WebSocket upgrade attempts
- Concurrent connections
- Connection pooling
- IPv4 address support
- Connection latency measurement

## Configuration

The Playwright configuration (`playwright.config.ts`) is set up to:

- Route all traffic through `socks5://localhost:1080`
- Use Chromium browser (expandable to Firefox and WebKit)
- Run tests in parallel
- Capture screenshots on failure
- Retain video on failure
- Generate HTML and JSON reports
- Set appropriate timeouts for proxy connections

## CI/CD Integration

These tests run automatically in GitHub Actions via `.github/workflows/playwright-proxy-tests.yml`:

- **Trigger**: Push to master, pull requests, daily at 2 AM UTC, manual dispatch
- **Platform**: Ubuntu (expandable to macOS)
- **Timeout**: 30 minutes
- **Artifacts**: Test results, reports, and proxy logs

## Troubleshooting

### Proxy Not Starting

If the proxy fails to start:

1. Check that you have a valid `BITPING_API_KEY`
2. Verify port 1080 is not already in use: `lsof -i :1080`
3. Check proxy logs for errors: `cat logs/p2proxy.log`

### Tests Timing Out

If tests are timing out:

1. Verify the proxy is running: `nc -z localhost 1080`
2. Check your internet connection
3. Increase timeout values in `playwright.config.ts`
4. Check if the P2P network is functioning correctly

### Connection Refused Errors

If you see "connection refused" errors:

1. Ensure P2Proxy is running and listening on port 1080
2. Verify the proxy authenticated successfully with Bitping
3. Check that the proxy has connected to peers

### Test Failures

If specific tests fail:

1. Check the test artifacts in `test-results/` and `playwright-report/`
2. Review screenshots and videos for visual debugging
3. Check proxy logs for any errors during the test
4. Run the test in headed mode to see what's happening: `npm run test:headed`

## Adding New Tests

To add new tests:

1. Create a new test file in `tests/` directory
2. Import Playwright test utilities: `import { test, expect } from '@playwright/test';`
3. Write your tests following the existing patterns
4. The SOCKS5 proxy configuration is automatically applied from `playwright.config.ts`
5. Run your tests to verify they work

Example:

```typescript
import { test, expect } from '@playwright/test';

test.describe('My New Tests', () => {
  test('should test something', async ({ page }) => {
    await page.goto('https://example.com/');
    await expect(page).toHaveTitle(/Example/);
  });
});
```

## Performance Considerations

- Tests use `domcontentloaded` instead of `load` to avoid waiting for all resources
- Timeouts are set conservatively for proxy connections
- Tests run in parallel for faster execution
- Network idle waits are used sparingly to avoid unnecessary delays

## Reporting

After running tests, view the HTML report:

```bash
npm run report
```

This opens an interactive report showing:
- Test results and status
- Screenshots and videos of failures
- Test duration and timing
- Error messages and stack traces

## Contributing

When adding new tests:

1. Follow the existing test structure and naming conventions
2. Add descriptive test names that explain what is being tested
3. Include comments for complex test logic
4. Handle timeouts gracefully (some sites may be slow)
5. Consider network flakiness and add appropriate retries/waits
6. Update this README if adding new test categories

## License

Same as P2Proxy project.
