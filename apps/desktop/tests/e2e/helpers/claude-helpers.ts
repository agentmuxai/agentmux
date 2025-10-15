/**
 * Helper utilities for testing Claude terminal interactions
 */

import { Page, expect } from '@playwright/test';

export interface ClaudeTestOptions {
  workdir?: string;
  agentId?: string;
  timeout?: number;
}

/**
 * Spawn a Claude agent in the test environment
 */
export async function spawnClaudeAgent(
  page: Page,
  options: ClaudeTestOptions = {}
): Promise<string> {
  const workdir = options.workdir || 'D:\\Code\\PythonProjects';
  const agentId = options.agentId || `claude-test-${Date.now()}`;

  console.log(`[Claude Helper] Spawning Claude agent: ${agentId}`);

  // Navigate to Claude instances tab/view
  const claudeTab = page.locator('text=/Claude|Instances/i').first();
  const hasClaudeTab = await claudeTab.isVisible().catch(() => false);

  if (hasClaudeTab) {
    await claudeTab.click();
    await page.waitForTimeout(500);
  }

  // Look for spawn controls
  const spawnButton = page.locator('button:has-text("Spawn"), button:has-text("Launch"), button:has-text("Start")').first();
  const hasSpawnButton = await spawnButton.isVisible().catch(() => false);

  if (hasSpawnButton) {
    // Fill in spawn form if it exists
    const idInput = page.locator('input[placeholder*="instance"], input[placeholder*="name"]').first();
    const hasIdInput = await idInput.isVisible().catch(() => false);

    if (hasIdInput) {
      await idInput.fill(agentId);
    }

    const workdirInput = page.locator('input[placeholder*="directory"], input[placeholder*="workspace"]').first();
    const hasWorkdirInput = await workdirInput.isVisible().catch(() => false);

    if (hasWorkdirInput) {
      await workdirInput.fill(workdir);
    }

    await spawnButton.click();
    console.log(`[Claude Helper] ✓ Clicked spawn button`);

    // Wait for agent to appear
    await page.waitForTimeout(2000);
  }

  console.log(`[Claude Helper] ✓ Claude agent spawned: ${agentId}`);
  return agentId;
}

/**
 * Confirm Claude's trust prompt automatically
 */
export async function confirmClaudeTrust(
  page: Page,
  timeout: number = 15000
): Promise<void> {
  console.log('[Claude Helper] Waiting for trust prompt...');

  try {
    // Wait for trust prompt to appear
    await page.waitForSelector('text=/Do you trust/i', { timeout });
    console.log('[Claude Helper] ✓ Trust prompt appeared');

    // Focus the terminal input
    const inputField = page.locator('.terminal-input').first();
    await inputField.click();

    // Ensure input is empty
    await inputField.fill('');

    // Press Enter to confirm (option 1 is pre-selected by cursor)
    await page.keyboard.press('Enter');
    console.log('[Claude Helper] ✓ Sent Enter key');

    // Wait for trust prompt to disappear
    await page.waitForSelector('text=/Do you trust/i', {
      state: 'hidden',
      timeout: 5000
    });

    console.log('[Claude Helper] ✓ Trust prompt confirmed');
  } catch (error) {
    console.error('[Claude Helper] ✗ Failed to confirm trust:', error);
    throw error;
  }
}

/**
 * Send a command to Claude and optionally wait for response
 */
export async function sendClaudeCommand(
  page: Page,
  command: string,
  waitForResponse: boolean = true
): Promise<string> {
  console.log(`[Claude Helper] Sending command: ${command}`);

  const inputField = page.locator('.terminal-input').first();

  // Focus and clear input
  await inputField.click();
  await inputField.fill('');

  // Type command
  await inputField.fill(command);

  // Press Enter
  await page.keyboard.press('Enter');

  console.log(`[Claude Helper] ✓ Command sent: ${command}`);

  if (waitForResponse) {
    // Wait a bit for response
    await page.waitForTimeout(1000);
  }

  // Get terminal output
  const terminalOutput = page.locator('.terminal-output').first();
  const output = await terminalOutput.textContent();

  return output || '';
}

/**
 * Wait for specific text to appear in Claude's terminal output
 */
export async function waitForClaudeOutput(
  page: Page,
  expectedText: string | RegExp,
  timeout: number = 10000
): Promise<void> {
  console.log(`[Claude Helper] Waiting for output: ${expectedText}`);

  const terminalOutput = page.locator('.terminal-output').first();

  if (typeof expectedText === 'string') {
    await expect(terminalOutput).toContainText(expectedText, { timeout });
  } else {
    await expect(terminalOutput).toContainText(expectedText, { timeout });
  }

  console.log(`[Claude Helper] ✓ Found expected output`);
}

/**
 * Click on the terminal output area (to test focus behavior)
 */
export async function clickTerminalOutput(page: Page): Promise<void> {
  console.log('[Claude Helper] Clicking terminal output area');

  const terminalOutput = page.locator('.terminal-output').first();
  await terminalOutput.click();

  console.log('[Claude Helper] ✓ Clicked terminal output');
}

/**
 * Verify the input field is focused
 */
export async function expectInputFocused(page: Page): Promise<void> {
  const inputField = page.locator('.terminal-input').first();
  await expect(inputField).toBeFocused();

  console.log('[Claude Helper] ✓ Input field is focused');
}

/**
 * Get the current scroll position of terminal output
 */
export async function getTerminalScrollTop(page: Page): Promise<number> {
  const terminalOutput = page.locator('.terminal-output').first();
  const scrollTop = await terminalOutput.evaluate(el => el.scrollTop);

  console.log(`[Claude Helper] Terminal scroll top: ${scrollTop}px`);

  return scrollTop;
}

/**
 * Send arrow key to Claude (for menu navigation)
 */
export async function sendArrowKey(
  page: Page,
  direction: 'up' | 'down'
): Promise<void> {
  const key = direction === 'up' ? 'ArrowUp' : 'ArrowDown';

  console.log(`[Claude Helper] Sending ${key}`);

  await page.keyboard.press(key);

  console.log(`[Claude Helper] ✓ ${key} sent`);
}
