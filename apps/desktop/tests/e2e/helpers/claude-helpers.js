/**
 * Helper utilities for Claude-specific E2E test interactions
 * Using WebdriverIO API with correct selectors for AgentMux UI
 */

import { getElement, clickElement, typeText, takeDebugScreenshot } from './tauri-app.js';

/**
 * Navigate to the Agents tab
 */
export async function navigateToAgentsTab() {
  console.log('[E2E] Navigating to Agents tab...');
  await clickElement('[data-testid="tab-agents"]');
  // Wait for the agents manager to load
  await getElement('.agents-manager', 5000);
  console.log('[E2E] ✓ Navigated to Agents tab');
}

/**
 * Spawn a Claude agent via the UI
 * @param {Object} options - Spawn options
 * @param {string} options.workspacePath - Working directory for Claude (REQUIRED)
 * @param {string} options.label - Optional label for the agent (UI only)
 * @returns {Promise<string>} - Agent label/instance name
 */
export async function spawnClaudeAgent(options = {}) {
  console.log('[Claude E2E] Spawning Claude agent...');

  // Navigate to Agents tab first
  await navigateToAgentsTab();

  // Fill in workspace path (required)
  if (!options.workspacePath) {
    throw new Error('workspacePath is required to spawn agent');
  }

  const workspaceInput = await getElement('input[placeholder*="Select workspace"]');
  await workspaceInput.setValue(options.workspacePath);

  // Fill in agent label (optional)
  if (options.label) {
    const labelInput = await getElement('input[placeholder*="MyAgent"]');
    await labelInput.setValue(options.label);
  }

  // Click spawn button (primary button in the spawn card)
  const spawnButtons = await $$('button.primary');
  await spawnButtons[0].click(); // First primary button is the spawn button

  // Wait for agent to appear in the list
  await browser.waitUntil(
    async () => {
      const agentCards = await $$('.agent-card');
      return agentCards.length > 0;
    },
    {
      timeout: 30000,
      timeoutMsg: 'Agent did not appear in the list within 30 seconds'
    }
  );

  const label = options.label || options.workspacePath.split(/[/\]/).filter(Boolean).pop() || 'Agent';
  console.log(`[Claude E2E] ✓ Claude agent spawned: ${label}`);
  return label;
}

/**
 * Select an agent from the agents list
 * @param {string} agentLabel - The label/instance name of the agent
 */
export async function selectAgent(agentLabel) {
  console.log(`[Claude E2E] Selecting agent: ${agentLabel}`);
  const agentCards = await $$('.agent-card');

  for (const card of agentCards) {
    const text = await card.getText();
    if (text.includes(agentLabel)) {
      await card.click();
      console.log(`[Claude E2E] ✓ Selected agent: ${agentLabel}`);
      // Wait for terminal to appear
      await getElement('.simple-terminal', 5000);
      return;
    }
  }

  throw new Error(`Agent not found: ${agentLabel}`);
}

/**
 * Wait for terminal to be connected
 * @param {number} timeout - Timeout in milliseconds
 */
export async function waitForTerminalConnected(timeout = 10000) {
  console.log('[Claude E2E] Waiting for terminal connection...');
  await browser.waitUntil(
    async () => {
      const statusDots = await $$('.terminal-header .status-dot.online');
      return statusDots.length > 0;
    },
    {
      timeout,
      timeoutMsg: 'Terminal did not connect within timeout'
    }
  );
  console.log('[Claude E2E] ✓ Terminal connected');
}

/**
 * Click on the terminal output area (for focus testing)
 */
export async function clickTerminalOutput() {
  console.log('[Claude E2E] Clicking terminal output area...');
  const output = await getElement('.terminal-output');
  await output.click();
  console.log('[Claude E2E] ✓ Terminal output clicked');
}

/**
 * Click on the terminal input field
 */
export async function clickTerminalInput() {
  console.log('[Claude E2E] Clicking terminal input...');
  const input = await getElement('.terminal-input');
  await input.click();
  console.log('[Claude E2E] ✓ Terminal input clicked');
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
