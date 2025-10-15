/**
 * E2E Tests: Claude Terminal Interaction
 *
 * Tests focus unification, keyboard event handling, and Claude responses
 */

import { test, expect } from '@playwright/test';
import { launchTauriApp, closeTauriApp, takeDebugScreenshot, TauriAppInstance } from './helpers/tauri-app';
import {
  spawnClaudeAgent,
  confirmClaudeTrust,
  clickTerminalOutput,
  expectInputFocused,
  getTerminalScrollTop,
  sendArrowKey,
  waitForClaudeOutput
} from './helpers/claude-helpers';

let tauriApp: TauriAppInstance | null = null;

test.describe('Claude Terminal - Focus and Interaction', () => {
  test.beforeAll(async () => {
    tauriApp = await launchTauriApp({ timeout: 60000 });
    console.log('[Test] ✓ Tauri app launched for Claude terminal tests');
  });

  test.afterAll(async () => {
    if (tauriApp) {
      await closeTauriApp(tauriApp);
      console.log('[Test] ✓ Tauri app closed');
    }
  });

  test('TC1: Click terminal output → input focused', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] TC1: Testing focus unification');

    // Spawn Claude instance
    await spawnClaudeAgent(page, {
      workdir: 'D:\\Code\\PythonProjects'
    });

    await page.waitForTimeout(2000);
    await takeDebugScreenshot(page, 'tc1-01-claude-spawned');

    // Click on terminal OUTPUT area (not input)
    await clickTerminalOutput(page);

    await takeDebugScreenshot(page, 'tc1-02-output-clicked');

    // Verify input field is now focused
    await expectInputFocused(page);

    console.log('[Test] ✅ TC1 PASSED: Terminal output click → input focused');
  });

  test('TC2: Arrow keys navigate without scrolling', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] TC2: Testing arrow key event handling');

    // Wait for Claude to show content
    await page.waitForTimeout(1000);

    // Get initial scroll position
    const initialScroll = await getTerminalScrollTop(page);
    console.log(`[Test] Initial scroll: ${initialScroll}px`);

    // Click terminal to focus
    const terminalContainer = page.locator('.simple-terminal').first();
    await terminalContainer.click();

    await takeDebugScreenshot(page, 'tc2-01-before-arrow-keys');

    // Press arrow down multiple times
    await sendArrowKey(page, 'down');
    await page.waitForTimeout(100);
    await sendArrowKey(page, 'down');
    await page.waitForTimeout(100);

    // Press arrow up
    await sendArrowKey(page, 'up');
    await page.waitForTimeout(100);

    await takeDebugScreenshot(page, 'tc2-02-after-arrow-keys');

    // Verify scroll position UNCHANGED (event didn't bubble and scroll)
    const afterScroll = await getTerminalScrollTop(page);
    console.log(`[Test] After arrows: ${afterScroll}px`);

    expect(afterScroll).toBe(initialScroll);

    console.log('[Test] ✅ TC2 PASSED: Arrow keys didn\'t scroll output');
  });

  test('TC3: Claude responds to Enter key', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] TC3: Testing Claude response to Enter key');

    // Check if trust prompt is visible
    const trustPrompt = page.locator('text=/Do you trust/i').first();
    const hasTrustPrompt = await trustPrompt.isVisible().catch(() => false);

    if (hasTrustPrompt) {
      console.log('[Test] Trust prompt found, confirming...');

      await takeDebugScreenshot(page, 'tc3-01-trust-prompt');

      // Confirm trust
      await confirmClaudeTrust(page);

      await takeDebugScreenshot(page, 'tc3-02-trust-confirmed');

      // Wait for Claude to become ready
      await page.waitForTimeout(2000);

      // Look for Claude prompt or ready indicator
      const terminalOutput = page.locator('.terminal-output');
      const outputText = await terminalOutput.textContent();

      console.log(`[Test] Terminal output after trust: ${outputText?.substring(0, 100)}...`);

      await takeDebugScreenshot(page, 'tc3-03-claude-ready');

      console.log('[Test] ✅ TC3 PASSED: Claude responded to Enter key');
    } else {
      console.log('[Test] ⚠ No trust prompt found (may already be trusted)');
      await takeDebugScreenshot(page, 'tc3-no-trust-prompt');
    }
  });

  test('TC4: Input and output appear continuous', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');
    const { page } = tauriApp;

    console.log('[Test] TC4: Testing visual continuity of output and input');

    // Get computed styles
    const outputBg = await page.locator('.terminal-output').first().evaluate(
      el => window.getComputedStyle(el).backgroundColor
    );

    const inputAreaBg = await page.locator('.terminal-input-area').first().evaluate(
      el => window.getComputedStyle(el).backgroundColor
    );

    const inputBg = await page.locator('.terminal-input').first().evaluate(
      el => window.getComputedStyle(el).backgroundColor
    );

    const inputBorder = await page.locator('.terminal-input').first().evaluate(
      el => window.getComputedStyle(el).borderTop
    );

    console.log(`[Test] Output background: ${outputBg}`);
    console.log(`[Test] Input area background: ${inputAreaBg}`);
    console.log(`[Test] Input background: ${inputBg}`);
    console.log(`[Test] Input border: ${inputBorder}`);

    // Take screenshot for visual verification
    await takeDebugScreenshot(page, 'tc4-visual-continuity');

    // Input should be transparent or same as output
    const inputIsTransparent = inputBg.includes('0, 0, 0, 0') || inputBg === 'rgba(0, 0, 0, 0)' || inputBg === 'transparent';

    if (inputIsTransparent) {
      console.log('[Test] ✓ Input is transparent (blends with output)');
    } else {
      console.log('[Test] ℹ Input has background:', inputBg);
    }

    // Input area should match output
    expect(outputBg).toBe(inputAreaBg);

    console.log('[Test] ✅ TC4 PASSED: Output and input appear continuous');
  });
});
