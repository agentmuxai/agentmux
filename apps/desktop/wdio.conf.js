/**
 * WebdriverIO Configuration for Tauri Desktop E2E Tests
 * Uses tauri-driver for WebView2 automation
 */

import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export const config = {
  //
  // ====================
  // Runner Configuration
  // ====================
  runner: 'local',

  //
  // ==================
  // Specify Test Files
  // ==================
  specs: [
    './tests/e2e/**/*.spec.js'
  ],

  // Patterns to exclude.
  exclude: [
    // 'path/to/excluded/files'
  ],

  //
  // ============
  // Capabilities
  // ============
  maxInstances: 1, // Run tests sequentially (one at a time)

  capabilities: [{
    maxInstances: 1,
    // Use tauri:options to tell tauri-driver which app to launch
    // Using debug build which has more recent frontend
    'tauri:options': {
      application: path.join(__dirname, 'src-tauri', 'target', 'debug', 'agentmux.exe'),
    },
  }],

  //
  // ===================
  // Test Configurations
  // ===================
  logLevel: 'info',
  bail: 0, // Don't stop after first failure

  baseUrl: 'http://localhost', // Not used for Tauri

  waitforTimeout: 30000, // 30 seconds for element waits
  connectionRetryTimeout: 120000, // 2 minutes for driver connection
  connectionRetryCount: 3,

  // Connect to tauri-driver
  hostname: '127.0.0.1',
  port: 4444, // Default tauri-driver port
  automationProtocol: 'webdriver',

  services: [],

  framework: 'mocha',
  reporters: ['spec'],

  //
  // =====
  // Hooks
  // =====
  /**
   * Gets executed before a worker process is spawned
   */
  onPrepare: async function () {
    const { spawn, spawnSync } = await import('child_process');

    // Build the Tauri app (debug mode for faster builds)
    console.log('[wdio] Building Tauri app (debug mode)...');
    const buildResult = spawnSync('cargo', ['build'], {
      cwd: path.join(__dirname, 'src-tauri'),
      stdio: 'inherit',
      env: {
        ...process.env,
        AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
      },
    });

    if (buildResult.status !== 0) {
      throw new Error('Tauri build failed');
    }
    console.log('[wdio] ✓ Tauri app built');

    // Start tauri-driver server
    console.log('[wdio] Starting tauri-driver server...');

    const tauriDriver = spawn('tauri-driver', [
      '--native-driver',
      path.join(__dirname, 'msedgedriver.exe')
    ], {
      stdio: 'inherit',
      env: {
        ...process.env,
        AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
      },
    });

    // Store process reference for cleanup
    global.tauriDriverProcess = tauriDriver;

    // Wait for tauri-driver to start
    await new Promise(resolve => setTimeout(resolve, 2000));

    console.log('[wdio] ✓ tauri-driver server started');
  },

  /**
   * Gets executed after all workers have shut down
   */
  onComplete: async function () {
    // Stop tauri-driver server
    if (global.tauriDriverProcess) {
      console.log('[wdio] Stopping tauri-driver server...');
      global.tauriDriverProcess.kill();
      console.log('[wdio] ✓ tauri-driver server stopped');
    }
  },

  /**
   * Gets executed before test execution begins
   */
  before: async function () {
    // Set up custom commands or global test utilities here
  },

  /**
   * Gets executed after all tests are done
   */
  after: async function () {
    // Cleanup after tests
  },

  //
  // =============
  // Mocha options
  // =============
  mochaOpts: {
    ui: 'bdd',
    timeout: 120000, // 2 minutes per test (Tauri startup can be slow)
  },
};
