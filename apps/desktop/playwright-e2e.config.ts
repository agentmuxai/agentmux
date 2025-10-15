import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright Configuration for E2E Testing
 * Tests against the built release binary, not dev server
 */

export default defineConfig({
  testDir: './tests/e2e',
  testMatch: ['**/*.spec.ts'],

  timeout: 120000, // 120 seconds per test

  expect: {
    timeout: 10000,
  },

  fullyParallel: false, // Run tests sequentially (only one app instance)

  workers: 1, // CRITICAL: Only one worker to avoid port conflicts

  forbidOnly: !!process.env.CI,

  retries: process.env.CI ? 2 : 0,

  reporter: [
    ['html'],
    ['list'],
    ['json', { outputFile: 'test-results/e2e-results.json' }]
  ],

  use: {
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },

  projects: [
    {
      name: 'e2e-desktop',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  // No webServer - tests launch the app themselves
});
