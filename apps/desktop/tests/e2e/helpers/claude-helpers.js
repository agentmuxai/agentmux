/**
 * Helper utilities for Claude-specific E2E test interactions
 * Using WebdriverIO API
 */

import { getElement, clickElement, typeText, takeDebugScreenshot } from './tauri-app.js';

/**
 * Spawn a Claude agent via the UI
 * @param {Object} options - Spawn options
 * @param {string} options.workdir - Working directory for Claude
 * @returns {Promise<string>} - Agent ID
 */
export async function spawnClaudeAgent(options = {}) {
  console.log('[Claude E2E] Spawning Claude agent...');

  // Click the "Spawn Claude" button or equivalent UI element
  // This is a placeholder - adjust selector based on actual UI
  await clickElement('button[data-testid="spawn-claude"]');

  // If workdir option provided, enter it
  if (options.workdir) {
    await typeText('input[name="workdir"]', options.workdir);
  }

  // Click confirm/submit
  await clickElement('button[type="submit"]');

  // Wait for agent to appear in the UI
  const agentElement = await getElement('.claude-agent', 30000);
  const agentId = await agentElement.getAttribute('data-agent-id');

  console.log(`[Claude E2E] ✓ Claude agent spawned: ${agentId}`);
  return agentId;
}

/**
 * Confirm Claude trust prompt (press Enter)
 */
export async function confirmClaudeTrust() {
  console.log('[Claude E2E] Confirming Claude trust prompt...');

  // Wait for trust prompt to appear
  const trustPrompt = await getElement('text=Do you trust', 30000);
  await trustPrompt.waitForDisplayed();

  // Focus the input and press Enter
  const input = await getElement('.terminal-input');
  await input.click();
  await browser.keys(['Enter']);

  // Wait for trust prompt to disappear
  await trustPrompt.waitForDisplayed({ reverse: true, timeout: 10000 });

  console.log('[Claude E2E] ✓ Trust confirmed');
}

/**
 * Send a command to Claude and optionally wait for response
 * @param {string} command - Command to send
 * @param {boolean} waitForResponse - Wait for Claude to respond
 * @returns {Promise<string>} - Response text (if waiting)
 */
export async function sendClaudeCommand(command, waitForResponse = true) {
  console.log(`[Claude E2E] Sending command: ${command}`);

  // Get the terminal input
  const input = await getElement('.terminal-input');

  // Type the command
  await input.setValue(command);

  // Press Enter
  await browser.keys(['Enter']);

  if (waitForResponse) {
    // Wait for response to appear (look for new output)
    await browser.pause(2000); // Give Claude time to respond

    // Get the terminal output
    const output = await getElement('.terminal-output');
    const responseText = await output.getText();

    console.log('[Claude E2E] ✓ Command sent, response received');
    return responseText;
  }

  console.log('[Claude E2E] ✓ Command sent');
  return '';
}

/**
 * Click on the terminal output area (not the input)
 */
export async function clickTerminalOutput() {
  console.log('[Claude E2E] Clicking terminal output area...');

  // Click on the output div (not the input)
  const output = await getElement('.terminal-output');
  await output.click();

  console.log('[Claude E2E] ✓ Terminal output clicked');
}

/**
 * Check if the terminal input is focused
 * @returns {Promise<boolean>} - True if focused
 */
export async function expectInputFocused() {
  console.log('[Claude E2E] Checking if input is focused...');

  const input = await getElement('.terminal-input');
  const isFocused = await input.isFocused();

  if (!isFocused) {
    throw new Error('Terminal input is not focused');
  }

  console.log('[Claude E2E] ✓ Input is focused');
  return true;
}

/**
 * Get the terminal output scroll position
 * @returns {Promise<number>} - Scroll top position
 */
export async function getTerminalScrollTop() {
  const output = await getElement('.terminal-output');

  // Execute script to get scrollTop
  const scrollTop = await browser.execute((el) => {
    return el.scrollTop;
  }, output);

  return scrollTop;
}

/**
 * Send an arrow key to the terminal
 * @param {'up' | 'down' | 'left' | 'right'} direction - Arrow key direction
 */
export async function sendArrowKey(direction) {
  console.log(`[Claude E2E] Sending arrow key: ${direction}`);

  const keyMap = {
    up: 'ArrowUp',
    down: 'ArrowDown',
    left: 'ArrowLeft',
    right: 'ArrowRight',
  };

  const key = keyMap[direction];
  if (!key) {
    throw new Error(`Invalid arrow key direction: ${direction}`);
  }

  await browser.keys([key]);

  console.log(`[Claude E2E] ✓ Arrow key sent: ${direction}`);
}

/**
 * Wait for Claude to produce output
 * @param {number} timeout - Timeout in milliseconds
 */
export async function waitForClaudeOutput(timeout = 30000) {
  console.log('[Claude E2E] Waiting for Claude output...');

  const output = await getElement('.terminal-output');

  // Wait for output to have non-empty text content
  await browser.waitUntil(
    async () => {
      const text = await output.getText();
      return text.trim().length > 0;
    },
    {
      timeout,
      timeoutMsg: 'Claude did not produce output within timeout',
    }
  );

  console.log('[Claude E2E] ✓ Claude output received');
}
