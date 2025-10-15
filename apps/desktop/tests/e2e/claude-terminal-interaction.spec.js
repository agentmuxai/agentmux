/**
 * E2E Tests: Claude Terminal Interaction
 *
 * Tests terminal focus, agent spawning, and interaction
 * Using WebdriverIO + tauri-driver
 */

import { expect } from '@wdio/globals';
import {
  waitForAppReady,
  takeDebugScreenshot,
} from './helpers/tauri-app.js';
import {
  spawnClaudeAgent,
  selectAgent,
  waitForTerminalConnected,
  clickTerminalOutput,
  clickTerminalInput,
  expectInputFocused,
  sendArrowKey,
} from './helpers/claude-helpers.js';

describe('AgentMux - Claude Terminal Interaction', () => {
  let agentLabel;

  before(async () => {
    // Wait for the Tauri app to be ready
    await waitForAppReady();
    console.log('[Test] ✓ Tauri app ready for tests');

    // Spawn Claude instance for all tests
    try {
      agentLabel = await spawnClaudeAgent({
        workspacePath: 'D:\\Code\\PythonProjects',
        label: 'TestAgent',
      });
      console.log(`[Test] ✓ Claude agent spawned: ${agentLabel}`);

      // Select the agent to show terminal
      await selectAgent(agentLabel);
      console.log('[Test] ✓ Agent selected');

      // Wait for terminal to connect
      await waitForTerminalConnected();
      console.log('[Test] ✓ Terminal connected');

      await takeDebugScreenshot('00-initial-state');
    } catch (error) {
      console.error('[Test] Failed to setup:', error);
      await takeDebugScreenshot('00-setup-failed');
      throw error;
    }
  });

  it('TC1: Terminal renders and shows connection', async () => {
    console.log('[Test] TC1: Testing terminal rendering');

    // Verify terminal elements exist
    const terminal = await $('.simple-terminal');
    expect(await terminal.isDisplayed()).toBe(true);

    const terminalOutput = await $('.terminal-output');
    expect(await terminalOutput.isDisplayed()).toBe(true);

    const terminalInput = await $('.terminal-input');
    expect(await terminalInput.isDisplayed()).toBe(true);

    // Verify connection status indicator
    const statusDot = await $('.terminal-header .status-dot.online');
    expect(await statusDot.isDisplayed()).toBe(true);

    await takeDebugScreenshot('tc1-terminal-rendered');

    console.log('[Test] ✅ TC1 PASSED: Terminal rendered and connected');
  });

  it('TC2: Click terminal output → input focused', async () => {
    console.log('[Test] TC2: Testing focus unification');

    await takeDebugScreenshot('tc2-01-before-click');

    // Click on terminal OUTPUT area (not input)
    await clickTerminalOutput();

    await takeDebugScreenshot('tc2-02-after-click');

    // Verify input field is now focused
    await expectInputFocused();

    console.log('[Test] ✅ TC2 PASSED: Terminal output click → input focused');
  });

  it('TC3: Arrow keys work in terminal input', async () => {
    console.log('[Test] TC3: Testing arrow key handling');

    // Ensure input is focused
    await clickTerminalInput();
    await expectInputFocused();

    await takeDebugScreenshot('tc3-01-before-arrow-keys');

    // Send arrow keys (these should be handled by terminal, not scroll page)
    await sendArrowKey('down');
    await browser.pause(100);
    await sendArrowKey('down');
    await browser.pause(100);
    await sendArrowKey('up');
    await browser.pause(100);

    await takeDebugScreenshot('tc3-02-after-arrow-keys');

    // Verify input is still focused (arrow keys didn't break focus)
    await expectInputFocused();

    console.log('[Test] ✅ TC3 PASSED: Arrow keys handled correctly');
  });

  it('TC4: Terminal auto-focuses when clicking anywhere', async () => {
    console.log('[Test] TC4: Testing auto-focus behavior');

    // Click somewhere else first (like the agent card)
    const agentCard = await $('.agent-card');
    await agentCard.click();
    await browser.pause(200);

    await takeDebugScreenshot('tc4-01-clicked-away');

    // Now click on the terminal container (not specifically input)
    const terminalContainer = await $('.simple-terminal');
    await terminalContainer.click();

    await takeDebugScreenshot('tc4-02-clicked-terminal');

    // Input should be focused
    await expectInputFocused();

    console.log('[Test] ✅ TC4 PASSED: Terminal auto-focus works');
  });

  it('TC5: Agent list shows spawned agent', async () => {
    console.log('[Test] TC5: Testing agent list display');

    // Agent card should exist and be selected
    const agentCard = await $('.agent-card.selected');
    expect(await agentCard.isDisplayed()).toBe(true);

    // Card should show agent label
    const cardText = await agentCard.getText();
    expect(cardText).toContain(agentLabel);

    // Card should show status
    expect(cardText).toContain('running');

    // Card should show PID
    expect(cardText).toMatch(/PID:/);

    await takeDebugScreenshot('tc5-agent-list');

    console.log('[Test] ✅ TC5 PASSED: Agent list displays correctly');
  });
});
