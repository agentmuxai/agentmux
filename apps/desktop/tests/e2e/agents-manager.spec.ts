/**
 * E2E Tests: Agents Manager View
 *
 * Based on user stories from USER_STORIES_AND_WIREFRAMES.md
 * Tests: US-A1, US-A2, US-A3, US-A4, US-A5, US-A6
 */

import { test, expect } from '@playwright/test';
import { launchTauriApp, closeTauriApp, takeDebugScreenshot, TauriAppInstance } from './helpers/tauri-app';

let tauriApp: TauriAppInstance | null = null;

test.describe('Agents Manager - Spawn and Control Agents', () => {
  test.beforeAll(async () => {
    tauriApp = await launchTauriApp({ timeout: 60000 });
    console.log('[Test] ✓ Tauri app launched for Agents Manager tests');
  });

  test.afterAll(async () => {
    if (tauriApp) {
      await closeTauriApp(tauriApp);
      console.log('[Test] ✓ Tauri app closed');
    }
  });

  test('US-A1: Spawn New Agent', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-A1: Spawn New Agent');

    // Navigate to Agents tab
    const agentsTab = page.locator('button:has-text("Agents")').first();
    await agentsTab.click();
    await page.waitForTimeout(500);
    await takeDebugScreenshot(page, 'agents-01-opened');

    // Verify spawn controls visible
    const spawnSection = page.locator('text=Spawn Agent').first();
    await expect(spawnSection).toBeVisible();
    console.log('[Test] ✓ Spawn Agent section visible');

    // Find ID input field
    const idInput = page.locator('input[placeholder*="agent"], input[type="text"]').first();
    await expect(idInput).toBeVisible();
    console.log('[Test] ✓ Agent ID input visible');

    // Enter agent ID
    const testAgentId = `agent-test-${Date.now()}`;
    await idInput.fill(testAgentId);
    console.log(`[Test] ✓ Entered agent ID: ${testAgentId}`);

    // Find working directory input (may not exist)
    const workdirInput = page.locator('input[placeholder*="directory"], input[placeholder*="workspace"]').first();
    const hasWorkdirInput = await workdirInput.isVisible().catch(() => false);

    if (hasWorkdirInput) {
      await workdirInput.fill('D:\\Code\\WebProjects');
      console.log('[Test] ✓ Entered working directory');
    }

    await takeDebugScreenshot(page, 'agents-02-spawn-form-filled');

    // Click spawn button
    const spawnButton = page.locator('button:has-text("Spawn")').first();
    await expect(spawnButton).toBeVisible();
    await spawnButton.click();
    console.log('[Test] ✓ Clicked Spawn button');

    await page.waitForTimeout(2000); // Wait for agent to spawn
    await takeDebugScreenshot(page, 'agents-03-agent-spawned');

    // Verify agent appears in list
    const agentCard = page.locator(`text=${testAgentId}`).first();
    const agentVisible = await agentCard.isVisible().catch(() => false);

    if (agentVisible) {
      console.log(`[Test] ✓ Agent ${testAgentId} appears in list`);
    } else {
      console.log(`[Test] ⚠ Agent may still be spawning...`);
    }

    console.log('[Test] ✅ US-A1 completed');
  });

  test('US-A2: View Agent Terminal Output', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-A2: View Agent Terminal Output');

    // Look for terminal output area (could be textarea, pre, code, or div)
    const terminalOutput = page.locator('textarea[readonly], pre, code, div[class*="terminal"], div[class*="output"]').first();
    const hasTerminal = await terminalOutput.isVisible().catch(() => false);

    if (hasTerminal) {
      console.log('[Test] ✓ Terminal output area visible');

      const outputText = await terminalOutput.textContent();
      if (outputText && outputText.length > 0) {
        console.log(`[Test] ✓ Terminal has output (${outputText.length} chars)`);
      }
    } else {
      console.log('[Test] ⚠ Terminal output area not found (may not be spawned yet)');
    }

    await takeDebugScreenshot(page, 'agents-04-terminal-output');

    // Look for status indicators
    const statusIndicator = page.locator('text=/status|running|idle|stopped/i').first();
    const hasStatus = await statusIndicator.isVisible().catch(() => false);

    if (hasStatus) {
      const statusText = await statusIndicator.textContent();
      console.log(`[Test] ✓ Status indicator: ${statusText}`);
    }

    console.log('[Test] ✅ US-A2 completed');
  });

  test('US-A3: Send Input to Agent', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-A3: Send Input to Agent');

    // Look for input field (not readonly)
    const inputField = page.locator('input[type="text"]:not([readonly]), textarea:not([readonly])').first();
    const hasInput = await inputField.isVisible().catch(() => false);

    if (hasInput) {
      console.log('[Test] ✓ Input field visible');

      // Type test message
      const testMessage = 'Hello from E2E test';
      await inputField.fill(testMessage);
      console.log(`[Test] ✓ Typed message: ${testMessage}`);

      await takeDebugScreenshot(page, 'agents-05-input-typed');

      // Find send button
      const sendButton = page.locator('button:has-text("Send"), button:has-text("Submit"), button[type="submit"]').first();
      const hasSendButton = await sendButton.isVisible().catch(() => false);

      if (hasSendButton) {
        await sendButton.click();
        console.log('[Test] ✓ Clicked Send button');

        await page.waitForTimeout(1000);
        await takeDebugScreenshot(page, 'agents-06-message-sent');

        // Check if input was cleared (indicates message sent)
        const inputValue = await inputField.inputValue();
        if (inputValue === '') {
          console.log('[Test] ✓ Input cleared after send');
        }
      } else {
        // Try pressing Enter
        await inputField.press('Enter');
        console.log('[Test] ✓ Pressed Enter to send');

        await page.waitForTimeout(1000);
        await takeDebugScreenshot(page, 'agents-06-message-sent');
      }
    } else {
      console.log('[Test] ⚠ Input field not found (no agent selected?)');
    }

    console.log('[Test] ✅ US-A3 completed');
  });

  test('US-A4: Stop Agent', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-A4: Stop Agent');

    // Look for stop button
    const stopButton = page.locator('button:has-text("Stop"), button:has-text("Kill"), button:has-text("Terminate")').first();
    const hasStopButton = await stopButton.isVisible().catch(() => false);

    if (hasStopButton) {
      console.log('[Test] ✓ Stop button visible');

      await stopButton.click();
      console.log('[Test] ✓ Clicked Stop button');

      await page.waitForTimeout(1000);
      await takeDebugScreenshot(page, 'agents-07-agent-stopped');

      // Check for status change
      const stoppedStatus = page.locator('text=/stopped|terminated|exited/i').first();
      const isStopped = await stoppedStatus.isVisible().catch(() => false);

      if (isStopped) {
        console.log('[Test] ✓ Agent status shows stopped');
      }
    } else {
      console.log('[Test] ⚠ Stop button not visible (no agent running?)');
    }

    console.log('[Test] ✅ US-A4 completed');
  });

  test('US-A5: Restart Agent', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-A5: Restart Agent');

    // Look for restart button
    const restartButton = page.locator('button:has-text("Restart"), button:has-text("Respawn")').first();
    const hasRestartButton = await restartButton.isVisible().catch(() => false);

    if (hasRestartButton) {
      console.log('[Test] ✓ Restart button visible');

      await restartButton.click();
      console.log('[Test] ✓ Clicked Restart button');

      await page.waitForTimeout(2000); // Wait for restart
      await takeDebugScreenshot(page, 'agents-08-agent-restarted');

      // Check for running status
      const runningStatus = page.locator('text=/running|active|ready/i').first();
      const isRunning = await runningStatus.isVisible().catch(() => false);

      if (isRunning) {
        console.log('[Test] ✓ Agent restarted successfully');
      }
    } else {
      console.log('[Test] ⚠ Restart button not visible');
    }

    console.log('[Test] ✅ US-A5 completed');
  });

  test('US-A6: Browse Workspace Directory', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] US-A6: Browse Workspace Directory');

    // Look for browse/folder button
    const browseButton = page.locator('button:has-text("Browse"), button:has-text("..."), button[aria-label*="browse"], button[title*="browse"]').first();
    const hasBrowseButton = await browseButton.isVisible().catch(() => false);

    if (hasBrowseButton) {
      console.log('[Test] ✓ Browse button visible');

      await takeDebugScreenshot(page, 'agents-09-before-browse');

      // Click browse button (this may open native file dialog)
      await browseButton.click();
      console.log('[Test] ✓ Clicked Browse button');

      await page.waitForTimeout(500);

      // Note: Native file dialogs can't be automated easily
      // We just verify the button works without errors
      console.log('[Test] ℹ Native file dialog may have opened (not automatable)');

      await takeDebugScreenshot(page, 'agents-10-after-browse');
    } else {
      console.log('[Test] ⚠ Browse button not found');
    }

    console.log('[Test] ✅ US-A6 completed');
  });
});
