/**
 * Helper utilities for launching and connecting to Tauri app in Playwright tests
 */

import { chromium, Browser, Page } from '@playwright/test';
import { spawn, ChildProcess } from 'child_process';
import * as path from 'path';

export interface TauriAppInstance {
  browser: Browser;
  page: Page;
  process: ChildProcess;
  debugPort: number;
}

/**
 * Launch the Tauri app with WebView2 remote debugging enabled
 * and connect Playwright to it
 */
export async function launchTauriApp(options?: {
  executablePath?: string;
  debugPort?: number;
  timeout?: number;
}): Promise<TauriAppInstance> {
  const debugPort = options?.debugPort || 9222;
  const timeout = options?.timeout || 30000;

  // Determine the executable path
  // Default: target/release/agentmux.exe or the provided path
  const executablePath = options?.executablePath ||
    path.join(process.cwd(), 'src-tauri', 'target', 'release', 'agentmux.exe');

  console.log(`[Tauri E2E] Launching Tauri app from: ${executablePath}`);
  console.log(`[Tauri E2E] WebView2 debugging port: ${debugPort}`);

  // Launch the Tauri app with remote debugging enabled
  const tauriProcess = spawn(executablePath, [], {
    env: {
      ...process.env,
      // Enable WebView2 remote debugging
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}`,
      // Disable any security restrictions for testing
      RUST_LOG: 'debug',
    },
    stdio: ['pipe', 'pipe', 'pipe'],
    shell: true,
  });

  // Log stdout/stderr for debugging
  tauriProcess.stdout?.on('data', (data) => {
    console.log(`[Tauri stdout] ${data.toString().trim()}`);
  });

  tauriProcess.stderr?.on('data', (data) => {
    console.error(`[Tauri stderr] ${data.toString().trim()}`);
  });

  tauriProcess.on('error', (error) => {
    console.error(`[Tauri E2E] Process error:`, error);
  });

  // Wait for the app to start and debugging port to be ready
  console.log(`[Tauri E2E] Waiting for debugging port to be ready...`);
  try {
    await waitForPort(debugPort, timeout);
    console.log(`[Tauri E2E] ✓ Debugging port ready`);
  } catch (error) {
    console.error(`[Tauri E2E] Failed to connect to debugging port:`, error);
    // Kill the process before re-throwing
    if (!tauriProcess.killed) {
      tauriProcess.kill('SIGKILL');
    }
    throw error;
  }

  // Connect Playwright to the WebView2 debugging port
  console.log(`[Tauri E2E] Connecting Playwright to debugging port...`);
  let browser: Browser;
  try {
    browser = await chromium.connectOverCDP(`http://localhost:${debugPort}`, {
      timeout,
    });
    console.log(`[Tauri E2E] ✓ Playwright connected`);
  } catch (error) {
    console.error(`[Tauri E2E] Failed to connect Playwright to CDP:`, error);
    // Kill the process before re-throwing
    if (!tauriProcess.killed) {
      tauriProcess.kill('SIGKILL');
    }
    throw error;
  }

  // Get the default context and page
  const contexts = browser.contexts();
  if (contexts.length === 0) {
    // Kill the process before throwing
    if (!tauriProcess.killed) {
      tauriProcess.kill('SIGKILL');
    }
    await browser.close().catch(() => {});
    throw new Error('No browser contexts found');
  }

  const pages = contexts[0].pages();
  if (pages.length === 0) {
    // Kill the process before throwing
    if (!tauriProcess.killed) {
      tauriProcess.kill('SIGKILL');
    }
    await browser.close().catch(() => {});
    throw new Error('No pages found in default context');
  }

  const page = pages[0];
  console.log(`[Tauri E2E] ✓ Connected to page: ${page.url()}`);

  return {
    browser,
    page,
    process: tauriProcess,
    debugPort,
  };
}

/**
 * Close the Tauri app and cleanup resources
 */
export async function closeTauriApp(instance: TauriAppInstance): Promise<void> {
  console.log(`[Tauri E2E] Closing Tauri app...`);

  try {
    // Close the browser connection
    await instance.browser.close();
    console.log(`[Tauri E2E] ✓ Browser connection closed`);
  } catch (error) {
    console.error(`[Tauri E2E] Error closing browser:`, error);
  }

  try {
    // Kill the Tauri process
    if (instance.process && !instance.process.killed) {
      instance.process.kill('SIGTERM');

      // Wait for process to exit
      await new Promise<void>((resolve) => {
        instance.process.on('exit', () => {
          console.log(`[Tauri E2E] ✓ Tauri process terminated`);
          resolve();
        });

        // Force kill after 5 seconds if not exited
        setTimeout(() => {
          if (!instance.process.killed) {
            console.log(`[Tauri E2E] Force killing Tauri process...`);
            instance.process.kill('SIGKILL');
          }
          resolve();
        }, 5000);
      });
    }
  } catch (error) {
    console.error(`[Tauri E2E] Error killing process:`, error);
  }
}

/**
 * Wait for a TCP port to be ready
 */
async function waitForPort(port: number, timeout: number): Promise<void> {
  const startTime = Date.now();

  while (Date.now() - startTime < timeout) {
    try {
      // Try to connect to the port
      const response = await fetch(`http://localhost:${port}/json/version`, {
        signal: AbortSignal.timeout(1000),
      });

      if (response.ok) {
        return; // Port is ready
      }
    } catch (error) {
      // Port not ready yet, continue waiting
    }

    // Wait 500ms before trying again
    await new Promise(resolve => setTimeout(resolve, 500));
  }

  throw new Error(`Timeout waiting for port ${port} to be ready after ${timeout}ms`);
}

/**
 * Helper to take a screenshot for debugging
 */
export async function takeDebugScreenshot(page: Page, name: string): Promise<void> {
  try {
    await page.screenshot({
      path: `test-results/screenshots/${name}-${Date.now()}.png`,
      fullPage: true,
    });
    console.log(`[Tauri E2E] ✓ Screenshot saved: ${name}`);
  } catch (error) {
    console.error(`[Tauri E2E] Failed to take screenshot:`, error);
  }
}
