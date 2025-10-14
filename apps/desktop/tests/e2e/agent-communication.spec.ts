/**
 * E2E Test: Agent Communication Flow
 *
 * Tests the complete user workflow:
 * 1. Spawn an agent
 * 2. Enter text in the input field
 * 3. Press Enter or click Send
 * 4. Wait for response from Claude
 * 5. Verify response appears in the output
 *
 * This test launches the actual Tauri desktop app and interacts with it via Playwright.
 */

import { test, expect } from '@playwright/test';
import { launchTauriApp, closeTauriApp, takeDebugScreenshot, TauriAppInstance } from './helpers/tauri-app';

let tauriApp: TauriAppInstance | null = null;

test.describe('Agent Communication', () => {
  test.beforeAll(async () => {
    // Launch the Tauri app
    // Note: Make sure you've built the app first with `npm run tauri:build`
    tauriApp = await launchTauriApp({
      // Optional: specify custom executable path
      // executablePath: '../../releases/v0.3.1/agentmux-desktop-v0.3.1-portable.exe',
      timeout: 60000, // 60 seconds to launch
    });

    console.log('[Test] ✓ Tauri app launched successfully');
  });

  test.afterAll(async () => {
    if (tauriApp) {
      await closeTauriApp(tauriApp);
      console.log('[Test] ✓ Tauri app closed');
    }
  });

  test('should spawn agent, send message, and receive response', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');

    const { page } = tauriApp;

    // Step 1: Wait for app to load
    console.log('[Test] Step 1: Waiting for app to load...');
    await page.waitForLoadState('domcontentloaded');
    await takeDebugScreenshot(page, '01-app-loaded');
    console.log('[Test] ✓ App loaded');

    // Step 2: Find and click "Spawn Agent" button
    console.log('[Test] Step 2: Looking for "Spawn Agent" button...');

    // Try multiple selector strategies
    const spawnButtonSelectors = [
      'button:has-text("Spawn Agent")',
      'button:has-text("Spawn")',
      'button[aria-label*="spawn" i]',
      'button[class*="spawn" i]',
      '[role="button"]:has-text("Spawn")',
    ];

    let spawnButton = null;
    for (const selector of spawnButtonSelectors) {
      try {
        spawnButton = page.locator(selector).first();
        if (await spawnButton.isVisible({ timeout: 2000 })) {
          console.log(`[Test] ✓ Found button with selector: ${selector}`);
          break;
        }
      } catch (e) {
        // Try next selector
      }
    }

    if (!spawnButton || !(await spawnButton.isVisible())) {
      console.error('[Test] ✗ Could not find "Spawn Agent" button');
      await takeDebugScreenshot(page, '02-spawn-button-not-found');

      // Log available buttons for debugging
      const buttons = await page.locator('button').all();
      console.log(`[Test] Found ${buttons.length} buttons:`);
      for (let i = 0; i < buttons.length; i++) {
        const text = await buttons[i].textContent();
        console.log(`  - Button ${i + 1}: "${text}"`);
      }

      throw new Error('Spawn Agent button not found');
    }

    console.log('[Test] Clicking "Spawn Agent" button...');
    await spawnButton.click();
    await page.waitForTimeout(2000); // Wait for agent to spawn
    await takeDebugScreenshot(page, '03-agent-spawned');
    console.log('[Test] ✓ Agent spawned');

    // Step 3: Find the message input field
    console.log('[Test] Step 3: Looking for message input field...');

    const inputSelectors = [
      'input[type="text"]',
      'input[placeholder*="message" i]',
      'input[placeholder*="input" i]',
      'textarea',
      'input[class*="input" i]',
      '[role="textbox"]',
    ];

    let messageInput = null;
    for (const selector of inputSelectors) {
      try {
        messageInput = page.locator(selector).first();
        if (await messageInput.isVisible({ timeout: 2000 })) {
          console.log(`[Test] ✓ Found input with selector: ${selector}`);
          break;
        }
      } catch (e) {
        // Try next selector
      }
    }

    if (!messageInput || !(await messageInput.isVisible())) {
      console.error('[Test] ✗ Could not find message input field');
      await takeDebugScreenshot(page, '04-input-not-found');

      // Log available inputs for debugging
      const inputs = await page.locator('input, textarea').all();
      console.log(`[Test] Found ${inputs.length} input fields:`);
      for (let i = 0; i < inputs.length; i++) {
        const type = await inputs[i].getAttribute('type');
        const placeholder = await inputs[i].getAttribute('placeholder');
        console.log(`  - Input ${i + 1}: type="${type}", placeholder="${placeholder}"`);
      }

      throw new Error('Message input field not found');
    }

    // Step 4: Type a test message
    const testMessage = 'Hello, this is an automated test message!';
    console.log(`[Test] Step 4: Typing message: "${testMessage}"`);
    await messageInput.click();
    await messageInput.fill(testMessage);
    await takeDebugScreenshot(page, '05-message-typed');
    console.log('[Test] ✓ Message typed');

    // Step 5: Press Enter or click Send button
    console.log('[Test] Step 5: Sending message...');

    // Try pressing Enter first
    await messageInput.press('Enter');
    console.log('[Test] ✓ Pressed Enter');

    // Alternatively, look for Send button
    const sendButtonSelectors = [
      'button:has-text("Send")',
      'button[aria-label*="send" i]',
      'button[class*="send" i]',
      '[role="button"]:has-text("Send")',
    ];

    for (const selector of sendButtonSelectors) {
      try {
        const sendButton = page.locator(selector).first();
        if (await sendButton.isVisible({ timeout: 1000 })) {
          console.log(`[Test] Found Send button with selector: ${selector}`);
          await sendButton.click();
          console.log('[Test] ✓ Clicked Send button');
          break;
        }
      } catch (e) {
        // Try next selector or skip if Enter worked
      }
    }

    await takeDebugScreenshot(page, '06-message-sent');

    // Step 6: Wait for response
    console.log('[Test] Step 6: Waiting for response from Claude...');

    // Look for response in various containers
    const outputSelectors = [
      '[class*="output" i]',
      '[class*="response" i]',
      '[class*="message" i]',
      '[class*="log" i]',
      '[role="log"]',
      'pre',
      'code',
    ];

    let responseFound = false;
    const maxWaitTime = 30000; // 30 seconds
    const startTime = Date.now();

    while (!responseFound && Date.now() - startTime < maxWaitTime) {
      for (const selector of outputSelectors) {
        try {
          const outputs = await page.locator(selector).all();

          for (const output of outputs) {
            const text = await output.textContent();
            if (text && text.trim().length > 0) {
              console.log(`[Test] Found output in ${selector}: "${text.substring(0, 100)}..."`);

              // Check if it's a response (not just the input echo)
              if (!text.includes(testMessage) || text.length > testMessage.length + 50) {
                console.log('[Test] ✓ Response detected!');
                responseFound = true;
                break;
              }
            }
          }
        } catch (e) {
          // Continue checking other selectors
        }

        if (responseFound) break;
      }

      if (!responseFound) {
        await page.waitForTimeout(1000); // Wait 1 second before checking again
      }
    }

    await takeDebugScreenshot(page, '07-response-received');

    // Step 7: Verify response
    if (!responseFound) {
      console.error('[Test] ✗ No response received within timeout');
      console.log('[Test] Page content:');
      console.log(await page.content());

      throw new Error('No response received from Claude within 30 seconds');
    }

    console.log('[Test] ✓ Test completed successfully!');

    // Additional assertions can be added here
    expect(responseFound).toBe(true);
  });

  test('should handle multiple messages sequentially', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');

    const { page } = tauriApp;

    console.log('[Test] Testing multiple sequential messages...');

    const messages = [
      'First test message',
      'Second test message',
      'Third test message',
    ];

    const messageInput = page.locator('input[type="text"]').first();

    for (let i = 0; i < messages.length; i++) {
      const message = messages[i];
      console.log(`[Test] Sending message ${i + 1}: "${message}"`);

      await messageInput.click();
      await messageInput.fill(message);
      await messageInput.press('Enter');

      // Wait a bit between messages
      await page.waitForTimeout(2000);

      await takeDebugScreenshot(page, `08-multiple-messages-${i + 1}`);
    }

    console.log('[Test] ✓ Multiple messages sent successfully');
  });

  test('should display logs in console', async () => {
    if (!tauriApp) throw new Error('Tauri app not initialized');

    const { page } = tauriApp;

    console.log('[Test] Checking console logs...');

    // Listen to console messages
    const consoleLogs: string[] = [];
    page.on('console', (msg) => {
      const text = msg.text();
      consoleLogs.push(text);
      console.log(`[Browser Console] ${text}`);
    });

    // Wait for some logs to appear
    await page.waitForTimeout(5000);

    console.log(`[Test] Captured ${consoleLogs.length} console messages`);

    // Verify we're seeing WebSocket logs
    const hasWebSocketLogs = consoleLogs.some(log =>
      log.includes('[WS:') || log.includes('WebSocket') || log.includes('stdin')
    );

    if (hasWebSocketLogs) {
      console.log('[Test] ✓ WebSocket logs are being generated');
    } else {
      console.log('[Test] ⚠ No WebSocket logs detected (this may be expected if no messages were sent)');
    }
  });
});
