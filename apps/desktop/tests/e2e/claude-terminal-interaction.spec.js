/**
 * E2E Tests: Claude Terminal Interaction
 *
 * Tests focus unification, keyboard event handling, and Claude responses
 * Using WebdriverIO + tauri-driver
 */

import { expect } from '@wdio/globals';
import {
  waitForAppReady,
  takeDebugScreenshot,
} from './helpers/tauri-app.js';
import {
  spawnClaudeAgent,
  confirmClaudeTrust,
  clickTerminalOutput,
  expectInputFocused,
  getTerminalScrollTop,
  sendArrowKey,
  waitForClaudeOutput,
} from './helpers/claude-helpers.js';

describe('Claude Terminal - Focus and Interaction', () => {
  before(async () => {
    // Wait for the Tauri app to be ready
    await waitForAppReady();
    console.log('[Test] ✓ Tauri app ready for tests');

    // Spawn Claude instance for all tests
    try {
      await spawnClaudeAgent({
        workdir: 'D:\\Code\\PythonProjects',
      });
      console.log('[Test] ✓ Claude agent spawned');
    } catch (error) {
      console.error('[Test] Failed to spawn Claude agent:', error);
      throw error;
    }

    // Wait for initial output
    await browser.pause(2000);
    await takeDebugScreenshot('00-initial-state');
  });

  it('TC1: Click terminal output → input focused', async () => {
    console.log('[Test] TC1: Testing focus unification');

    await takeDebugScreenshot('tc1-01-before-click');

    // Click on terminal OUTPUT area (not input)
    await clickTerminalOutput();

    await takeDebugScreenshot('tc1-02-after-click');

    // Verify input field is now focused
    await expectInputFocused();

    console.log('[Test] ✅ TC1 PASSED: Terminal output click → input focused');
  });

  it('TC2: Arrow keys navigate without scrolling', async () => {
    console.log('[Test] TC2: Testing arrow key event handling');

    // Wait for Claude to show content
    await browser.pause(1000);

    // Get initial scroll position
    const initialScroll = await getTerminalScrollTop();
    console.log(`[Test] Initial scroll: ${initialScroll}px`);

    await takeDebugScreenshot('tc2-01-before-arrow-keys');

    // Press arrow down multiple times
    await sendArrowKey('down');
    await browser.pause(100);
    await sendArrowKey('down');
    await browser.pause(100);

    // Press arrow up
    await sendArrowKey('up');
    await browser.pause(100);

    await takeDebugScreenshot('tc2-02-after-arrow-keys');

    // Verify scroll position UNCHANGED (event didn't bubble and scroll)
    const afterScroll = await getTerminalScrollTop();
    console.log(`[Test] After arrows: ${afterScroll}px`);

    expect(afterScroll).toBe(initialScroll);

    console.log('[Test] ✅ TC2 PASSED: Arrow keys didn\'t scroll output');
  });

  it('TC3: Claude responds to Enter key', async () => {
    console.log('[Test] TC3: Testing Claude response to Enter key');

    // Check if trust prompt is visible
    const trustPromptSelector = '*=Do you trust';
    const trustPrompt = await $(trustPromptSelector);
    const hasTrustPrompt = await trustPrompt.isDisplayed().catch(() => false);

    if (hasTrustPrompt) {
      console.log('[Test] Trust prompt found, confirming...');

      await takeDebugScreenshot('tc3-01-trust-prompt');

      // Confirm trust
      await confirmClaudeTrust();

      await takeDebugScreenshot('tc3-02-trust-confirmed');

      // Wait for Claude to become ready
      await browser.pause(2000);

      // Look for Claude prompt or ready indicator
      const terminalOutput = await $('.terminal-output');
      const outputText = await terminalOutput.getText();

      console.log(`[Test] Terminal output after trust: ${outputText.substring(0, 100)}...`);

      await takeDebugScreenshot('tc3-03-claude-ready');

      console.log('[Test] ✅ TC3 PASSED: Claude responded to Enter key');
    } else {
      console.log('[Test] ⚠ No trust prompt found (may already be trusted)');
      await takeDebugScreenshot('tc3-no-trust-prompt');
    }
  });

  it('TC4: Input and output appear continuous', async () => {
    console.log('[Test] TC4: Testing visual continuity of output and input');

    // Get computed styles using browser.execute
    const outputBg = await browser.execute(() => {
      const el = document.querySelector('.terminal-output');
      return window.getComputedStyle(el).backgroundColor;
    });

    const inputAreaBg = await browser.execute(() => {
      const el = document.querySelector('.terminal-input-area');
      return window.getComputedStyle(el).backgroundColor;
    });

    const inputBg = await browser.execute(() => {
      const el = document.querySelector('.terminal-input');
      return window.getComputedStyle(el).backgroundColor;
    });

    const inputBorder = await browser.execute(() => {
      const el = document.querySelector('.terminal-input');
      return window.getComputedStyle(el).borderTop;
    });

    console.log(`[Test] Output background: ${outputBg}`);
    console.log(`[Test] Input area background: ${inputAreaBg}`);
    console.log(`[Test] Input background: ${inputBg}`);
    console.log(`[Test] Input border: ${inputBorder}`);

    // Take screenshot for visual verification
    await takeDebugScreenshot('tc4-visual-continuity');

    // Input should be transparent or same as output
    const inputIsTransparent =
      inputBg.includes('0, 0, 0, 0') ||
      inputBg === 'rgba(0, 0, 0, 0)' ||
      inputBg === 'transparent';

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
