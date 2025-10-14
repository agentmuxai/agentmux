import { test as base, Page } from '@playwright/test';
import { MainPage } from '../pages/MainPage';

/**
 * Custom Playwright fixtures for AgentMux testing
 *
 * Provides dependency injection for page objects and test setup/teardown.
 */

type AgentMuxFixtures = {
  mainPage: MainPage;
};

/**
 * Extended test fixture with page objects
 */
export const test = base.extend<AgentMuxFixtures>({
  /**
   * Main page fixture - automatically instantiated for each test
   */
  mainPage: async ({ page }, use) => {
    const mainPage = new MainPage(page);
    await use(mainPage);
  },
});

/**
 * Export expect from @playwright/test for consistency
 */
export { expect } from '@playwright/test';
