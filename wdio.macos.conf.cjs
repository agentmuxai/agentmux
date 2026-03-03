/**
 * macOS WDIO config — runs tests against Vite dev server with mocked Tauri IPC.
 * No tauri-driver needed. Uses devtools protocol to connect to Chrome.
 *
 * Usage:
 *   1. Start Vite: npx vite --config vite.config.tauri.ts
 *   2. Run tests:  npx wdio run wdio.macos.conf.cjs
 *
 * Or use the npm script: npm run test:e2e:macos
 */
const { version } = require('./package.json')
const baseConfig = require('./wdio.conf.cjs').config

exports.config = {
  ...baseConfig,

  // Override: no tauri-driver needed
  onPrepare: () => {},
  onComplete: () => {},

  // Use devtools protocol instead of WebDriver
  automationProtocol: 'devtools',
  services: ['devtools'],

  capabilities: [{
    maxInstances: 1,
    browserName: 'chrome',
    'goog:chromeOptions': {
      args: ['--auto-open-devtools-for-tabs']
    }
  }],

  // Point at Vite dev server
  baseUrl: 'http://localhost:5173',

  // Inject Tauri IPC mocks before each test
  before: async function () {
    await browser.url('/')
    await browser.execute(function (appVersion) {
      if (!window.__TAURI_INTERNALS__) {
        // Mutable zoom state for get/set_zoom_factor
        let mockZoomFactor = 1.0

        window.__TAURI_INTERNALS__ = {
          invoke: async (cmd, args) => {
            const mocks = {
              get_auth_key: 'test-auth-key-0123456789abcdef',
              get_is_dev: true,
              get_platform: 'darwin',
              get_user_name: 'testuser',
              get_host_name: 'testhost',
              get_data_dir: '/tmp/agentmux-test',
              get_config_dir: '/tmp/agentmux-test/config',
              get_env: '',
              get_about_modal_details: {
                version: appVersion,
                buildTime: '202602160000',
              },
              get_backend_endpoints: {
                ws: 'ws://127.0.0.1:0/ws',
                web: 'http://127.0.0.1:0',
              },
              get_docsite_url: 'https://docs.agentmux.dev',
              get_window_label: 'main',
              is_main_window: true,
              list_windows: ['main'],
              open_new_window: 'window-2',
              minimize_window: null,
              maximize_window: null,
              close_window: null,
              toggle_devtools: null,
              show_context_menu: null,
              set_window_init_status: null,
              fe_log: null,
              register_global_webview_keys: null,
              update_wco: null,
              set_keyboard_chord_mode: null,
              set_waveai_open: null,
            }

            // Dynamic zoom commands
            if (cmd === 'get_zoom_factor') {
              return mockZoomFactor
            }
            if (cmd === 'set_zoom_factor') {
              let factor = args?.factor ?? 1.0
              // Clamp to valid range
              factor = Math.max(0.5, Math.min(3.0, factor))
              mockZoomFactor = factor
              return null
            }

            if (cmd in mocks) return mocks[cmd]
            console.warn(`[WDIO mock] Unhandled Tauri command: ${cmd}`, args)
            return null
          },
        }
      }
    }, version)
  },
}
