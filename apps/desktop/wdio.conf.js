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
    // tauri-driver expects 'tauri' as browserName
    browserName: 'tauri',

    // Path to the Tauri executable (release build)
    'tauri:options': {
      application: path.join(__dirname, 'src-tauri', 'target', 'release', 'agentmux.exe'),
    },

    // WebView2 specific options
    'ms:edgeOptions': {
      // Args passed to WebView2
      args: [],
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

  services: [
    // Custom service to start tauri-driver
    [
      'custom',
      {
        async onPrepare() {
          const { spawn } = await import('child_process');

          // Start tauri-driver server
          console.log('[wdio] Starting tauri-driver server...');

          const tauriDriver = spawn('tauri-driver', ['--native-driver', 'msedgedriver'], {
            stdio: 'inherit',
          });

          // Store process reference for cleanup
          global.tauriDriverProcess = tauriDriver;

          // Wait for tauri-driver to start
          await new Promise(resolve => setTimeout(resolve, 2000));

          console.log('[wdio] ✓ tauri-driver server started');
        },

        async onComplete() {
          // Stop tauri-driver server
          if (global.tauriDriverProcess) {
            console.log('[wdio] Stopping tauri-driver server...');
            global.tauriDriverProcess.kill();
            console.log('[wdio] ✓ tauri-driver server stopped');
          }
        },
      },
    ],
  ],

  framework: 'mocha',
  reporters: ['spec'],

  //
  // =====
  // Hooks
  // =====
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
