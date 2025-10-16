/**
 * Helper utilities for Tauri app E2E tests using WebdriverIO + tauri-driver
 *
 * NOTE: With tauri-driver, we don't need to manually launch the app or manage connections.
 * The driver handles all of that automatically. These helpers are just convenience wrappers.
 */

/**
 * Wait for the Tauri app to be ready
 * @param {number} timeout - Timeout in milliseconds (default: 10000)
 */
export async function waitForAppReady(timeout = 10000) {
  console.log('[Tauri E2E] Waiting for app to be ready...');

  // Wait for the main window to be available
  await browser.waitUntil(
    async () => {
      const windows = await browser.getWindowHandles();
      return windows.length > 0;
    },
    {
      timeout,
      timeoutMsg: 'Tauri app did not start within timeout',
    }
  );

  // Switch to the main window
  const windows = await browser.getWindowHandles();
  await browser.switchToWindow(windows[0]);

  console.log('[Tauri E2E] ✓ App ready');
}

/**
 * Take a screenshot for debugging
 * @param {string} name - Screenshot name
 */
export async function takeDebugScreenshot(name) {
  try {
    await browser.saveScreenshot(`./test-results/screenshots/${name}-${Date.now()}.png`);
    console.log(`[Tauri E2E] ✓ Screenshot saved: ${name}`);
  } catch (error) {
    console.error(`[Tauri E2E] Failed to take screenshot:`, error);
  }
}

/**
 * Helper to get an element with a timeout
 * @param {string} selector - CSS selector
 * @param {number} timeout - Timeout in milliseconds
 */
export async function getElement(selector, timeout = 10000) {
  const element = await $(selector);
  await element.waitForExist({ timeout });
  return element;
}

/**
 * Helper to click an element safely
 * @param {string} selector - CSS selector
 */
export async function clickElement(selector) {
  const element = await getElement(selector);
  await element.waitForClickable();
  await element.click();
}

/**
 * Helper to type text into an input
 * @param {string} selector - CSS selector for input
 * @param {string} text - Text to type
 */
export async function typeText(selector, text) {
  const element = await getElement(selector);
  await element.waitForClickable();
  await element.setValue(text);
}
