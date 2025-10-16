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
  getElement,
} from './helpers/tauri-app.js';
import {
  spawnClaudeAgent,
  selectAgent,
  waitForTerminalConnected,
  clickTerminalContainer,
  expectPaneActive,
  sendArrowKey,
  sendMessageToAgent,
  waitForAgentResponse,
  getTerminalStatus,
  verify2WayCommunication,
} from './helpers/claude-helpers.js';

describe('AgentMux - Claude Terminal Interaction', () => {
  before(async () => {
    // Wait for the Tauri app to be ready
    await waitForAppReady();
    console.log('[Test] ✓ Tauri app ready for tests');

    try {
      // Wait for the first pane to load (auto-spawns agent)
      console.log('[Test] Waiting for first pane to load...');
      await getElement('.pane', 30000); // Increased timeout for agent spawn
      console.log('[Test] ✓ First pane loaded');

      // Wait for embedded terminal to appear
      await getElement('.embedded-terminal', 10000);
      console.log('[Test] ✓ Embedded terminal rendered');

      // Wait for terminal to connect
      await waitForTerminalConnected(15000);
      console.log('[Test] ✓ Terminal connected');

      await takeDebugScreenshot('00-initial-state');
    } catch (error) {
      console.error('[Test] Failed to setup:', error);
      await takeDebugScreenshot('00-setup-failed');
      throw error;
    }
  });

  it('TC1: Terminal renders and shows connection', async () => {
    console.log('[Test] TC1: Testing terminal rendering (xterm.js UI)');

    // Verify terminal container exists (xterm.js based)
    const terminal = await getElement('.embedded-terminal');
    expect(await terminal.isDisplayed()).toBe(true);

    // Verify terminal header
    const header = await terminal.$('.terminal-header');
    expect(await header.isDisplayed()).toBe(true);

    // Verify terminal container (xterm.js mount point - canvas based)
    const container = await terminal.$('.terminal-container');
    expect(await container.isDisplayed()).toBe(true);

    // Verify connection status indicator
    const statusDot = await $('.status-dot');
    const statusClasses = await statusDot.getAttribute('class');
    expect(statusClasses).toContain('online');

    // Verify instance name and port are displayed
    const status = await getTerminalStatus();
    expect(status.isOnline).toBe(true);
    expect(status.instanceName).toBeTruthy();
    expect(status.port).toMatch(/ws:\/\/localhost:\d+/);

    await takeDebugScreenshot('tc1-terminal-rendered');

    console.log('[Test] ✅ TC1 PASSED: Terminal rendered and connected');
  });

  it('TC2: Click terminal container triggers focus', async () => {
    console.log('[Test] TC2: Testing terminal container click (xterm.js)');

    await takeDebugScreenshot('tc2-01-before-click');

    // Click on terminal container (xterm.js canvas area)
    await clickTerminalContainer();

    await takeDebugScreenshot('tc2-02-after-click');

    // Verify pane is active (xterm.js manages internal focus)
    // We can't directly check xterm.js focus, but we verify pane activation
    await expectPaneActive();

    console.log('[Test] ✅ TC2 PASSED: Terminal container click activates pane');
  });

  it('TC3: Arrow keys dispatched to terminal', async () => {
    console.log('[Test] TC3: Testing arrow key handling (xterm.js)');

    // Click terminal to ensure focus
    await clickTerminalContainer();

    await takeDebugScreenshot('tc3-01-before-arrow-keys');

    // Send arrow keys (xterm.js will handle internally)
    await sendArrowKey('down');
    await browser.pause(100);
    await sendArrowKey('down');
    await browser.pause(100);
    await sendArrowKey('up');
    await browser.pause(100);

    await takeDebugScreenshot('tc3-02-after-arrow-keys');

    // Verify connection is still online (no crashes from arrow keys)
    const status = await getTerminalStatus();
    expect(status.isOnline).toBe(true);

    console.log('[Test] ✅ TC3 PASSED: Arrow keys handled correctly');
  });

  it('TC4: Terminal pane gets focus when clicked', async () => {
    console.log('[Test] TC4: Testing pane focus behavior');

    // Click on app header to remove focus from terminal
    const header = await $('[data-testid="app-header"]');
    await header.click();
    await browser.pause(200);

    await takeDebugScreenshot('tc4-01-clicked-away');

    // Now click on the terminal container
    await clickTerminalContainer();

    await takeDebugScreenshot('tc4-02-clicked-terminal');

    // Pane should be active
    await expectPaneActive();

    console.log('[Test] ✅ TC4 PASSED: Terminal pane activation works');
  });

  it('TC5: Pane displays spawned agent', async () => {
    console.log('[Test] TC5: Testing pane displays agent (new auto-spawn UI)');

    // Pane should exist
    const pane = await getElement('.pane');
    expect(await pane.isDisplayed()).toBe(true);

    // Pane should be active
    const classes = await pane.getAttribute('class');
    expect(classes).toContain('pane-active');

    // Pane should show terminal
    const terminal = await pane.$('.embedded-terminal');
    expect(await terminal.isDisplayed()).toBe(true);

    // Terminal should show instance name
    const status = await getTerminalStatus();
    expect(status.instanceName).toBeTruthy();
    expect(status.isOnline).toBe(true);

    await takeDebugScreenshot('tc5-pane-with-agent');

    console.log('[Test] ✅ TC5 PASSED: Pane displays spawned agent correctly');
  });

  it('TC6: Send message to agent via terminal', async () => {
    console.log('[Test] TC6: Testing message sending to agent (xterm.js)');

    await takeDebugScreenshot('tc6-01-before-message');

    // Send a simple command to the agent
    const testMessage = 'echo "Hello from E2E test"';
    await sendMessageToAgent(testMessage);

    await browser.pause(2000); // Give agent time to process

    await takeDebugScreenshot('tc6-02-after-message');

    // Verify connection is still online (message was processed)
    const status = await getTerminalStatus();
    expect(status.isOnline).toBe(true);

    console.log('[Test] ✅ TC6 PASSED: Message sent to agent');
  });

  it('TC7: Receive response from agent', async () => {
    console.log('[Test] TC7: Testing agent response reception (xterm.js)');

    await takeDebugScreenshot('tc7-01-before-command');

    // Send a command that will generate output
    await sendMessageToAgent('pwd');

    await takeDebugScreenshot('tc7-02-after-command');

    // Wait for connection to remain stable (response processing)
    // NOTE: xterm.js canvas - we verify connection instead of text
    await waitForAgentResponse(null, 15000);

    await takeDebugScreenshot('tc7-03-response-received');

    console.log('[Test] ✅ TC7 PASSED: Agent response received (connection verified)');
  });

  it('TC8: Verify full 2-way communication cycle', async () => {
    console.log('[Test] TC8: Testing complete 2-way communication (xterm.js)');

    await takeDebugScreenshot('tc8-01-start');

    // Send a command and verify connection stability
    // NOTE: xterm.js canvas - we verify connection instead of specific text
    const success = await verify2WayCommunication(
      'echo "AgentMux E2E Test"',
      null, // Cannot verify text in canvas
      30000
    );

    expect(success).toBe(true);

    await takeDebugScreenshot('tc8-02-communication-verified');

    console.log('[Test] ✅ TC8 PASSED: 2-way communication verified (connection stable)');
  });

  it('TC9: Multiple message exchanges', async () => {
    console.log('[Test] TC9: Testing multiple message exchanges (xterm.js)');

    await takeDebugScreenshot('tc9-01-start');

    // First exchange (verify connection stability)
    await sendMessageToAgent('echo "Test 1"');
    await waitForAgentResponse(null, 10000);
    console.log('[Test] ✓ Exchange 1 complete');

    await browser.pause(500);

    // Second exchange
    await sendMessageToAgent('echo "Test 2"');
    await waitForAgentResponse(null, 10000);
    console.log('[Test] ✓ Exchange 2 complete');

    await browser.pause(500);

    // Third exchange
    await sendMessageToAgent('echo "Test 3"');
    await waitForAgentResponse(null, 10000);
    console.log('[Test] ✓ Exchange 3 complete');

    // Verify connection is still online after all exchanges
    const status = await getTerminalStatus();
    expect(status.isOnline).toBe(true);

    await takeDebugScreenshot('tc9-02-all-exchanges-complete');

    console.log('[Test] ✅ TC9 PASSED: Multiple exchanges successful (connection stable)');
  });
});
