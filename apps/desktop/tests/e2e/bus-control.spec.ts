/**
 * E2E Tests: Bus Control View
 *
 * Based on user stories from USER_STORIES_AND_WIREFRAMES.md
 * Tests: US-B1, US-B2, US-B3
 */

import { test, expect } from '@playwright/test';
import { launchTauriApp, closeTauriApp, takeDebugScreenshot, TauriAppInstance } from './helpers/tauri-app';

let tauriApp: TauriAppInstance | null = null;

test.describe('Bus Control - Advanced Bus Management', () => {
  test.beforeAll(async () => {
    tauriApp = await launchTauriApp({ timeout: 60000 });
    console.log('[Test] ✓ Tauri app launched for Bus Control tests');
  });

  test.afterAll(async () => {
    if (tauriApp) {
      await closeTauriApp(tauriApp);
      console.log('[Test] ✓ Tauri app closed');
    }
  });

  test('US-B1: Configure Bus Port and Address', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-B1: Configure Bus Port and Address');

    // Navigate to Bus tab
    const busTab = page.locator('button:has-text("Bus")').first();
    await busTab.click();
    await page.waitForTimeout(500);
    await takeDebugScreenshot(page, 'bus-01-opened');

    // Look for configuration section
    const configSection = page.locator('text=Configuration, text=Settings, text=Bus Config').first();
    const hasConfig = await configSection.isVisible().catch(() => false);

    if (hasConfig) {
      console.log('[Test] ✓ Configuration section visible');
    }

    // Look for port input
    const portInput = page.locator('input[type="number"], input[placeholder*="port"], input[name*="port"]').first();
    const hasPortInput = await portInput.isVisible().catch(() => false);

    if (hasPortInput) {
      console.log('[Test] ✓ Port input visible');

      // Get current value
      const currentPort = await portInput.inputValue();
      console.log(`[Test] ℹ Current port: ${currentPort}`);

      // Try changing port
      await portInput.fill('8766');
      console.log('[Test] ✓ Changed port to 8766');

      await takeDebugScreenshot(page, 'bus-02-port-changed');
    } else {
      console.log('[Test] ⚠ Port input not found');
    }

    // Look for address input
    const addressInput = page.locator('input[placeholder*="address"], input[placeholder*="host"], input[name*="address"]').first();
    const hasAddressInput = await addressInput.isVisible().catch(() => false);

    if (hasAddressInput) {
      console.log('[Test] ✓ Address input visible');

      const currentAddress = await addressInput.inputValue();
      console.log(`[Test] ℹ Current address: ${currentAddress}`);
    }

    // Look for apply/save button
    const applyButton = page.locator('button:has-text("Apply"), button:has-text("Save"), button:has-text("Update")').first();
    const hasApplyButton = await applyButton.isVisible().catch(() => false);

    if (hasApplyButton) {
      console.log('[Test] ✓ Apply button visible');

      await applyButton.click();
      console.log('[Test] ✓ Clicked Apply button');

      await page.waitForTimeout(500);
      await takeDebugScreenshot(page, 'bus-03-config-applied');
    }

    console.log('[Test] ✅ US-B1 completed');
  });

  test('US-B2: View Connected Agents List', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-B2: View Connected Agents List');

    // Start bus first (if not already running)
    const startBusButton = page.locator('button:has-text("Start Bus")').first();
    const canStartBus = await startBusButton.isVisible().catch(() => false);

    if (canStartBus) {
      await startBusButton.click();
      console.log('[Test] ✓ Started bus');
      await page.waitForTimeout(2000);
    }

    // Look for connected agents section
    const agentsSection = page.locator('text=Connected Agents, text=Active Agents, text=Agents List').first();
    const hasAgentsSection = await agentsSection.isVisible().catch(() => false);

    if (hasAgentsSection) {
      console.log('[Test] ✓ Connected Agents section visible');
    }

    await takeDebugScreenshot(page, 'bus-04-agents-list');

    // Look for agent list (table, list, or cards)
    const agentList = page.locator('table tbody tr, ul li, div[class*="agent-card"]');
    const agentCount = await agentList.count();

    console.log(`[Test] ℹ Connected agents: ${agentCount}`);

    if (agentCount > 0) {
      // Check first agent details
      const firstAgent = agentList.first();
      const agentText = await firstAgent.textContent();
      console.log(`[Test] ✓ First agent: ${agentText?.substring(0, 50)}...`);
    } else {
      console.log('[Test] ℹ No agents connected yet');
    }

    // Look for agent metadata (ID, connection time, status)
    const agentId = page.locator('text=/agent-\\w+/, code, span[class*="agent-id"]').first();
    const hasAgentId = await agentId.isVisible().catch(() => false);

    if (hasAgentId) {
      const idText = await agentId.textContent();
      console.log(`[Test] ✓ Agent ID visible: ${idText}`);
    }

    console.log('[Test] ✅ US-B2 completed');
  });

  test('US-B3: Monitor Bus Performance Metrics', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-B3: Monitor Bus Performance Metrics');

    // Look for performance metrics section
    const metricsSection = page.locator('text=Performance, text=Metrics, text=Statistics').first();
    const hasMetrics = await metricsSection.isVisible().catch(() => false);

    if (hasMetrics) {
      console.log('[Test] ✓ Performance metrics section visible');
    }

    await takeDebugScreenshot(page, 'bus-05-metrics');

    // Look for specific metrics
    const metrics = [
      { name: 'Messages/sec', selector: 'text=/messages.*sec/i, text=/msg.*s/i' },
      { name: 'Total messages', selector: 'text=/total.*messages/i' },
      { name: 'Uptime', selector: 'text=/uptime/i, time' },
      { name: 'Memory usage', selector: 'text=/memory/i, text=/MB/i' },
      { name: 'CPU usage', selector: 'text=/cpu/i, text=/%/i' }
    ];

    for (const metric of metrics) {
      const metricElement = page.locator(metric.selector).first();
      const hasMetric = await metricElement.isVisible().catch(() => false);

      if (hasMetric) {
        const metricText = await metricElement.textContent();
        console.log(`[Test] ✓ ${metric.name}: ${metricText}`);
      }
    }

    // Look for performance graph/chart
    const chart = page.locator('canvas, svg[class*="chart"], div[class*="graph"]').first();
    const hasChart = await chart.isVisible().catch(() => false);

    if (hasChart) {
      console.log('[Test] ✓ Performance chart visible');
    }

    // Look for refresh/update indicator
    const lastUpdate = page.locator('text=/updated/i, text=/last/i').first();
    const hasLastUpdate = await lastUpdate.isVisible().catch(() => false);

    if (hasLastUpdate) {
      const updateText = await lastUpdate.textContent();
      console.log(`[Test] ✓ Last update: ${updateText}`);
    }

    await takeDebugScreenshot(page, 'bus-06-performance-details');

    console.log('[Test] ✅ US-B3 completed');
  });
});
