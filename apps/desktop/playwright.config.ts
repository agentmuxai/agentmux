import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright Configuration for AgentMux Desktop Application Testing
 *
 * This configuration enables testing of the Tauri desktop application using
 * Chrome DevTools Protocol (CDP) for browser automation.
 */
export default defineConfig({
  testDir: './tests/ui',

  // Run tests in files in parallel
  fullyParallel: true,

  // Fail the build on CI if you accidentally left test.only in the source code
  forbidOnly: !!process.env.CI,

  // Retry on CI only
  retries: process.env.CI ? 2 : 0,

  // Opt out of parallel tests on CI
  workers: process.env.CI ? 1 : undefined,

  // Reporter to use
  reporter: [
    ['html'],
    ['list'],
    ['json', { outputFile: 'test-results/results.json' }],
  ],

  // Shared settings for all the projects below
  use: {
    // Collect trace on failure for debugging
    trace: 'retain-on-failure',

    // Screenshot on failure
    screenshot: 'only-on-failure',

    // Video on failure
    video: 'retain-on-failure',

    // Connect to Tauri app via CDP
    connectOptions: {
      wsEndpoint: 'ws://localhost:9222',
    },
  },

  // Configure projects for major browsers
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  // Start Tauri dev server with CDP enabled before running tests
  webServer: {
    command: 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" npm run tauri:dev',
    url: 'http://localhost:9222',
    timeout: 120 * 1000,
    reuseExistingServer: !process.env.CI,
  },
});
