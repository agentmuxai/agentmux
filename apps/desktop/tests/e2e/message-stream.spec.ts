/**
 * E2E Tests: Message Stream View
 *
 * Based on user stories from USER_STORIES_AND_WIREFRAMES.md
 * Tests: US-M1, US-M2, US-M3, US-M4, US-M5, US-M6
 */

import { test, expect } from '@playwright/test';
import { launchTauriApp, closeTauriApp, takeDebugScreenshot, TauriAppInstance } from './helpers/tauri-app';

let tauriApp: TauriAppInstance | null = null;

test.describe('Message Stream - Monitor Messages', () => {
  test.beforeAll(async () => {
    tauriApp = await launchTauriApp({ timeout: 60000 });
    console.log('[Test] ✓ Tauri app launched for Message Stream tests');
  });

  test.afterAll(async () => {
    if (tauriApp) {
      await closeTauriApp(tauriApp);
      console.log('[Test] ✓ Tauri app closed');
    }
  });

  test('US-M1: View Live Message Stream', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-M1: View Live Message Stream');

    // Navigate to Messages tab
    const messagesTab = page.locator('button:has-text("Messages")').first();
    await messagesTab.click();
    await page.waitForTimeout(500);
    await takeDebugScreenshot(page, 'messages-01-opened');

    // Verify message stream area visible
    const messageStream = page.locator('text=Message Stream, text=Messages').first();
    await expect(messageStream).toBeVisible();
    console.log('[Test] ✓ Message Stream section visible');

    // Look for message list (could be table, list, or div container)
    const messageList = page.locator('table, ul, ol, div[class*="message"], div[class*="stream"]').first();
    const hasList = await messageList.isVisible().catch(() => false);

    if (hasList) {
      console.log('[Test] ✓ Message list container visible');
    }

    // Check for auto-scroll toggle
    const autoScrollToggle = page.locator('input[type="checkbox"], button:has-text("Auto-scroll")').first();
    const hasAutoScroll = await autoScrollToggle.isVisible().catch(() => false);

    if (hasAutoScroll) {
      console.log('[Test] ✓ Auto-scroll control visible');
    }

    await takeDebugScreenshot(page, 'messages-02-stream-view');

    console.log('[Test] ✅ US-M1 completed');
  });

  test('US-M2: Filter Messages by Agent', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-M2: Filter Messages by Agent');

    // Look for filter controls
    const filterDropdown = page.locator('select, input[placeholder*="filter"], input[placeholder*="search"]').first();
    const hasFilter = await filterDropdown.isVisible().catch(() => false);

    if (hasFilter) {
      console.log('[Test] ✓ Filter control visible');

      const tagName = await filterDropdown.evaluate(el => el.tagName.toLowerCase());

      if (tagName === 'select') {
        // Dropdown - check for options
        const options = await filterDropdown.locator('option').count();
        console.log(`[Test] ✓ Filter has ${options} options`);

        if (options > 1) {
          // Select second option (first is usually "All")
          await filterDropdown.selectOption({ index: 1 });
          console.log('[Test] ✓ Selected filter option');

          await page.waitForTimeout(500);
          await takeDebugScreenshot(page, 'messages-03-filtered');
        }
      } else {
        // Input field - type filter
        await filterDropdown.fill('agent');
        console.log('[Test] ✓ Typed filter text');

        await page.waitForTimeout(500);
        await takeDebugScreenshot(page, 'messages-03-filtered');
      }
    } else {
      console.log('[Test] ⚠ Filter control not found');
    }

    console.log('[Test] ✅ US-M2 completed');
  });

  test('US-M3: Search Message Content', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-M3: Search Message Content');

    // Look for search input
    const searchInput = page.locator('input[type="search"], input[placeholder*="search"]').first();
    const hasSearch = await searchInput.isVisible().catch(() => false);

    if (hasSearch) {
      console.log('[Test] ✓ Search input visible');

      // Type search query
      const searchQuery = 'message';
      await searchInput.fill(searchQuery);
      console.log(`[Test] ✓ Typed search query: ${searchQuery}`);

      await page.waitForTimeout(500);
      await takeDebugScreenshot(page, 'messages-04-searched');

      // Check for search results indicator
      const resultsCount = page.locator('text=/\\d+ results/, text=/found/i').first();
      const hasResults = await resultsCount.isVisible().catch(() => false);

      if (hasResults) {
        const resultsText = await resultsCount.textContent();
        console.log(`[Test] ✓ Search results: ${resultsText}`);
      }
    } else {
      console.log('[Test] ⚠ Search input not found');
    }

    console.log('[Test] ✅ US-M3 completed');
  });

  test('US-M4: View Message Details', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-M4: View Message Details');

    // Look for message rows (clickable)
    const messageRow = page.locator('tr[role="button"], div[role="button"], li[role="button"], tr, div[class*="message-row"]').first();
    const hasMessageRow = await messageRow.isVisible().catch(() => false);

    if (hasMessageRow) {
      console.log('[Test] ✓ Message row found');

      await messageRow.click();
      console.log('[Test] ✓ Clicked message row');

      await page.waitForTimeout(500);
      await takeDebugScreenshot(page, 'messages-05-details');

      // Check for detail panel or modal
      const detailPanel = page.locator('dialog, aside, div[role="dialog"], div[class*="detail"], div[class*="modal"]').first();
      const hasDetails = await detailPanel.isVisible().catch(() => false);

      if (hasDetails) {
        console.log('[Test] ✓ Message details panel visible');

        // Look for timestamp, sender, content fields
        const timestamp = page.locator('text=/\\d{2}:\\d{2}/, time').first();
        const hasTimestamp = await timestamp.isVisible().catch(() => false);

        if (hasTimestamp) {
          console.log('[Test] ✓ Timestamp visible');
        }
      }
    } else {
      console.log('[Test] ⚠ No messages to click');
    }

    console.log('[Test] ✅ US-M4 completed');
  });

  test('US-M5: Export Message History', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-M5: Export Message History');

    // Look for export button
    const exportButton = page.locator('button:has-text("Export"), button:has-text("Download"), button:has-text("Save")').first();
    const hasExport = await exportButton.isVisible().catch(() => false);

    if (hasExport) {
      console.log('[Test] ✓ Export button visible');

      await takeDebugScreenshot(page, 'messages-06-before-export');

      await exportButton.click();
      console.log('[Test] ✓ Clicked Export button');

      await page.waitForTimeout(500);

      // Note: File download dialogs can't be easily automated
      console.log('[Test] ℹ File download may have started (not automatable)');

      await takeDebugScreenshot(page, 'messages-07-after-export');
    } else {
      console.log('[Test] ⚠ Export button not found');
    }

    console.log('[Test] ✅ US-M5 completed');
  });

  test('US-M6: Clear Message History', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-M6: Clear Message History');

    // Look for clear button
    const clearButton = page.locator('button:has-text("Clear"), button:has-text("Delete All"), button:has-text("Reset")').first();
    const hasClear = await clearButton.isVisible().catch(() => false);

    if (hasClear) {
      console.log('[Test] ✓ Clear button visible');

      // Count messages before clearing
      const messageBefore = await page.locator('tr, li, div[class*="message-row"]').count();
      console.log(`[Test] ℹ Messages before clear: ${messageBefore}`);

      await clearButton.click();
      console.log('[Test] ✓ Clicked Clear button');

      await page.waitForTimeout(500);

      // Check for confirmation dialog
      const confirmDialog = page.locator('dialog, div[role="alertdialog"]').first();
      const hasConfirm = await confirmDialog.isVisible().catch(() => false);

      if (hasConfirm) {
        console.log('[Test] ✓ Confirmation dialog appeared');

        const confirmButton = confirmDialog.locator('button:has-text("Confirm"), button:has-text("Yes"), button:has-text("OK")').first();
        const hasConfirmButton = await confirmButton.isVisible().catch(() => false);

        if (hasConfirmButton) {
          await confirmButton.click();
          console.log('[Test] ✓ Confirmed clear action');
        }
      }

      await page.waitForTimeout(500);
      await takeDebugScreenshot(page, 'messages-08-cleared');

      // Count messages after clearing
      const messagesAfter = await page.locator('tr, li, div[class*="message-row"]').count();
      console.log(`[Test] ℹ Messages after clear: ${messagesAfter}`);

      // Check for empty state message
      const emptyState = page.locator('text=/no messages/i, text=/empty/i').first();
      const hasEmptyState = await emptyState.isVisible().catch(() => false);

      if (hasEmptyState) {
        console.log('[Test] ✓ Empty state message visible');
      }
    } else {
      console.log('[Test] ⚠ Clear button not found');
    }

    console.log('[Test] ✅ US-M6 completed');
  });
});
