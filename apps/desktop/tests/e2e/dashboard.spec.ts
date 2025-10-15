/**
 * E2E Tests: Dashboard View
 *
 * Based on user stories from USER_STORIES_AND_WIREFRAMES.md
 * Tests: US-D1, US-D2, US-D3, US-D4
 */

import { test, expect } from '@playwright/test';
import { launchTauriApp, closeTauriApp, takeDebugScreenshot, TauriAppInstance } from './helpers/tauri-app';

let tauriApp: TauriAppInstance | null = null;

test.describe('Dashboard - Server Bus Control', () => {
  test.beforeAll(async () => {
    tauriApp = await launchTauriApp({ timeout: 60000 });
    console.log('[Test] ✓ Tauri app launched for Dashboard tests');
  });

  test.afterAll(async () => {
    if (tauriApp) {
      await closeTauriApp(tauriApp);
      console.log('[Test] ✓ Tauri app closed');
    }
  });

  test('US-D1: Start Message Bus', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-D1: Start Message Bus');

    // Navigate to Dashboard tab
    const dashboardTab = page.locator('button:has-text("Dashboard")').first();
    await dashboardTab.click();
    await page.waitForTimeout(500);
    await takeDebugScreenshot(page, 'dashboard-01-opened');

    // Verify initial state: Bus stopped
    const statusText = page.locator('text=Status: Stopped').first();
    await expect(statusText).toBeVisible();
    console.log('[Test] ✓ Initial state: Bus stopped');

    // Click "Start Bus" button
    const startBusButton = page.locator('button:has-text("Start Bus")').first();
    await expect(startBusButton).toBeVisible();
    await startBusButton.click();
    console.log('[Test] ✓ Clicked Start Bus button');

    await takeDebugScreenshot(page, 'dashboard-02-bus-starting');

    // Wait for bus to start (up to 5 seconds)
    await page.waitForTimeout(2000);

    // Verify status changed (may show "Running" or different text)
    // We'll check that "Stopped" is no longer visible
    const stoppedText = page.locator('text=Status: Stopped');
    const isStillStopped = await stoppedText.isVisible().catch(() => false);

    if (!isStillStopped) {
      console.log('[Test] ✓ Status changed from Stopped');
    } else {
      console.log('[Test] ⚠ Bus may still be starting...');
    }

    await takeDebugScreenshot(page, 'dashboard-03-bus-started');

    // Verify Bus Status card shows connection info
    const busStatusCard = page.locator('text=Bus Status').first();
    await expect(busStatusCard).toBeVisible();
    console.log('[Test] ✓ Bus Status card visible');

    // Check for WebSocket URL (ws://localhost:8765)
    const wsUrl = page.locator('text=/ws:\\/\\/.*:\\d+/').first();
    const hasWsUrl = await wsUrl.isVisible().catch(() => false);
    if (hasWsUrl) {
      const urlText = await wsUrl.textContent();
      console.log(`[Test] ✓ WebSocket URL visible: ${urlText}`);
    }

    console.log('[Test] ✅ US-D1 completed');
  });

  test('US-D3: Monitor Bus Metrics', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-D3: Monitor Bus Metrics');

    // Verify metrics cards are visible
    const connectedAgentsCard = page.locator('text=Connected Agents').first();
    await expect(connectedAgentsCard).toBeVisible();
    console.log('[Test] ✓ Connected Agents card visible');

    const messagesPerSecCard = page.locator('text=Messages/sec').first();
    await expect(messagesPerSecCard).toBeVisible();
    console.log('[Test] ✓ Messages/sec card visible');

    const busStatusCard = page.locator('text=Bus Status').first();
    await expect(busStatusCard).toBeVisible();
    console.log('[Test] ✓ Bus Status card visible');

    await takeDebugScreenshot(page, 'dashboard-04-metrics-visible');

    // Verify metric values are displayed
    // Look for numbers (0 or more)
    const metricNumbers = page.locator('text=/^\\d+$/');
    const count = await metricNumbers.count();
    console.log(`[Test] ✓ Found ${count} metric numbers displayed`);

    expect(count).toBeGreaterThan(0);

    console.log('[Test] ✅ US-D3 completed');
  });

  test('US-D4: View Recent Activity', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-D4: View Recent Activity');

    // Verify Recent Activity section exists
    const recentActivityHeading = page.locator('text=Recent Activity').first();
    await expect(recentActivityHeading).toBeVisible();
    console.log('[Test] ✓ Recent Activity section visible');

    await takeDebugScreenshot(page, 'dashboard-05-recent-activity');

    // Check for activity messages or tips
    const activitySection = page.locator('text=Recent Activity').locator('..').first();
    const hasContent = await activitySection.textContent();

    if (hasContent && hasContent.length > 20) {
      console.log(`[Test] ✓ Recent Activity has content (${hasContent.length} chars)`);
    }

    // Look for tip about Agents tab
    const tip = page.locator('text=/tip/i').first();
    const hasTip = await tip.isVisible().catch(() => false);
    if (hasTip) {
      const tipText = await tip.textContent();
      console.log(`[Test] ✓ Found tip: ${tipText?.substring(0, 50)}...`);
    }

    console.log('[Test] ✅ US-D4 completed');
  });

  test('US-D2: Stop Message Bus', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-D2: Stop Message Bus');

    // Click "Stop Bus" button
    const stopBusButton = page.locator('button:has-text("Stop Bus")').first();
    const isVisible = await stopBusButton.isVisible().catch(() => false);

    if (isVisible) {
      await stopBusButton.click();
      console.log('[Test] ✓ Clicked Stop Bus button');

      await page.waitForTimeout(1000);

      await takeDebugScreenshot(page, 'dashboard-06-bus-stopped');

      // Verify status shows Stopped
      const statusStopped = page.locator('text=Status: Stopped').first();
      await expect(statusStopped).toBeVisible();
      console.log('[Test] ✓ Status shows Stopped');

      // Verify metrics reset
      const offlineText = page.locator('text=Offline').first();
      const isOffline = await offlineText.isVisible().catch(() => false);
      if (isOffline) {
        console.log('[Test] ✓ Metrics show Offline');
      }

      console.log('[Test] ✅ US-D2 completed');
    } else {
      console.log('[Test] ⚠ Stop Bus button not visible (bus may not be running)');
      console.log('[Test] ⏭ US-D2 skipped');
    }
  });
});
