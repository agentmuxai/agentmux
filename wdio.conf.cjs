const os = require('os')
const path = require('path')
const { spawn } = require('child_process')

// Keep track of the tauri-driver process
let tauriDriver

// Platform detection
const isWindows = os.platform() === 'win32'

// Platform-aware configuration
const getAppBinary = () => {
  const basePath = path.resolve(__dirname, 'src-tauri/target/release/agentmux')
  return isWindows ? `${basePath}.exe` : basePath
}

exports.config = {
  runner: 'local',
  specs: ['./test/specs/**/*.e2e.js'],
  maxInstances: 1,
  hostname: 'localhost',
  port: 4444,
  path: '/',
  capabilities: [
    {
      maxInstances: 1,
      'tauri:options': {
        application: getAppBinary()
      },
      // Required for EdgeDriver 117+ on Windows
      // Temporarily disabled due to WebDriver validation errors with EdgeDriver 144
      // ...(isWindows && { webviewOptions: {} })
    }
  ],
  logLevel: 'info',
  bail: 0,
  baseUrl: 'http://localhost',
  waitforTimeout: 10000,
  connectionRetryTimeout: 120000,
  connectionRetryCount: 3,
  services: [],
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 60000
  },

  /**
   * Gets executed before test execution begins. At this point you can access all global
   * variables, such as `browser`. It is the perfect place to define custom commands.
   */
  before: function (capabilities, specs) {
    // Add custom commands if needed
  },

  /**
   * Gets executed before the suite starts
   */
  beforeSuite: function (suite) {
    console.log(`Starting suite: ${suite.title}`)
  },

  /**
   * Function to be executed before a test (in Mocha/Jasmine) starts.
   */
  beforeTest: function (test, context) {
    console.log(`Running test: ${test.title}`)
  },

  /**
   * Hook that gets executed after the suite has ended
   */
  afterSuite: function (suite) {
    console.log(`Finished suite: ${suite.title}`)
  },

  /**
   * Gets executed after all tests are done. You still have access to all global variables from
   * the test.
   */
  after: function (result, capabilities, specs) {
    // do something
  },

  /**
   * Ensure tauri-driver is running before tests start
   */
  onPrepare: function (config, capabilities) {
    console.log('Starting tauri-driver...')
    const tauriDriverPath = path.resolve(os.homedir(), '.cargo', 'bin', 'tauri-driver')

    tauriDriver = spawn(tauriDriverPath, ['--port', '4444'], {
      stdio: ['ignore', 'pipe', 'pipe']
    })

    tauriDriver.stdout.on('data', (data) => {
      console.log(`tauri-driver: ${data}`)
    })

    tauriDriver.stderr.on('data', (data) => {
      console.error(`tauri-driver error: ${data}`)
    })

    // Give tauri-driver time to start
    return new Promise((resolve) => setTimeout(resolve, 2000))
  },

  /**
   * Clean up tauri-driver after tests complete
   */
  onComplete: function (exitCode, config, capabilities, results) {
    console.log('Stopping tauri-driver...')
    if (tauriDriver) {
      tauriDriver.kill()
    }
  }
}
