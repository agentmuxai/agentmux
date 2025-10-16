# Tauri E2E Testing: tauri-driver vs WebdriverIO Research Report

**Date:** 2025-10-15
**Context:** After migrating from Playwright to tauri-driver + WebdriverIO, tests still fail
**Blocker:** msedgedriver trying to launch agentmux.exe as if it were Microsoft Edge browser
**Purpose:** Understand the difference between tauri-driver and WebdriverIO, correct integration approach, and Tauri's official best practices

---

## Executive Summary

### The Fundamental Misunderstanding

We've been treating **tauri-driver** and **WebdriverIO** as separate alternatives, when in fact:

- **tauri-driver** is a **WebDriver server** (backend)
- **WebdriverIO** is a **WebDriver client** (frontend/test framework)
- They are meant to **work together**, not as alternatives

### What We Did Wrong

Our `wdio.conf.js` configuration is using **`browserName: 'msedge'`** and pointing directly at `msedgedriver.exe`, which causes WebdriverIO to treat our Tauri app as if it were Microsoft Edge browser.

### What We Should Do

Use the **`tauri:options`** capability with the **`application`** property to tell tauri-driver which Tauri binary to launch.

---

## Part 1: What is tauri-driver?

### Definition

**tauri-driver** is a cross-platform WebDriver server specifically designed for Tauri applications.

### What It Does

1. **Wraps native WebDriver servers**:
   - **Windows**: Wraps `msedgedriver.exe` (WebView2 driver)
   - **Linux**: Wraps `WebKitWebDriver` (WebKit driver)
   - **macOS**: Not supported (no WKWebView driver available)

2. **Provides a unified interface**: Abstracts away platform differences, giving you a single API regardless of whether you're on Windows (WebView2) or Linux (WebKit)

3. **Manages application lifecycle**: Launches your Tauri app, connects to its WebView, and handles cleanup

4. **Speaks WebDriver protocol**: Implements the W3C WebDriver standard, so any WebDriver client can connect to it

### Installation

```bash
cargo install tauri-driver
```

### Running

```bash
# With native driver in PATH
tauri-driver

# With explicit native driver path
tauri-driver --native-driver /path/to/msedgedriver.exe

# Default port is 4444
tauri-driver --port 4445
```

### What It Does NOT Do

- **Does NOT provide a testing framework** (no test runner, no assertions, no test specs)
- **Does NOT replace test clients** like WebdriverIO, Selenium, Playwright
- **Does NOT directly interact with your tests**

Think of it as: **tauri-driver is to Tauri what chromedriver is to Chrome**

---

## Part 2: What is WebdriverIO?

### Definition

**WebdriverIO (WDIO)** is a Node.js test automation framework that provides a WebDriver client with a full testing suite.

### What It Does

1. **WebDriver Client**: Connects to WebDriver servers (chromedriver, geckodriver, tauri-driver, etc.)

2. **Test Framework Integration**: Works with Mocha, Jasmine, Cucumber

3. **Test Runner**: Manages test execution, parallelization, reporting

4. **API for Writing Tests**: Provides `browser`, `$`, `expect`, etc. for test specs

5. **Configuration Management**: `wdio.conf.js` controls all aspects of testing

6. **Lifecycle Hooks**: `onPrepare`, `beforeSession`, `afterSession`, etc.

### Installation

```bash
npm install --save-dev @wdio/cli @wdio/local-runner @wdio/mocha-framework @wdio/spec-reporter webdriverio
```

### What It Does NOT Do

- **Does NOT launch Tauri apps directly**
- **Does NOT understand Tauri-specific features** without tauri-driver
- **Does NOT replace tauri-driver**

Think of it as: **WebdriverIO is to test automation what Jest is to unit testing**

---

## Part 3: How They Work Together (Official Tauri Approach)

### The Correct Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     WebdriverIO (Test Client)               │
│  - Runs test specs                                          │
│  - Provides test framework (Mocha)                          │
│  - Sends WebDriver commands                                 │
└─────────────────┬───────────────────────────────────────────┘
                  │ WebDriver Protocol (HTTP/JSON)
                  │ Port 4444 (default)
                  ↓
┌─────────────────────────────────────────────────────────────┐
│                 tauri-driver (WebDriver Server)             │
│  - Receives WebDriver commands                              │
│  - Translates to platform-specific calls                    │
│  - Launches Tauri application                               │
│  - Manages app lifecycle                                    │
└─────────────────┬───────────────────────────────────────────┘
                  │
                  ↓
┌─────────────────────────────────────────────────────────────┐
│            msedgedriver.exe (Windows)                       │
│            WebKitWebDriver (Linux)                          │
│  - Native WebView automation                                │
│  - Platform-specific driver                                 │
└─────────────────┬───────────────────────────────────────────┘
                  │
                  ↓
┌─────────────────────────────────────────────────────────────┐
│                   Your Tauri App                            │
│                  (agentmux.exe)                             │
└─────────────────────────────────────────────────────────────┘
```

### The Flow

1. **WebdriverIO** starts and reads `wdio.conf.js`
2. **`onPrepare` hook** builds the Tauri app (`cargo build --release`)
3. **`beforeSession` hook** spawns `tauri-driver` process
4. **tauri-driver** starts listening on port 4444
5. **WebdriverIO** connects to `localhost:4444`
6. **WebdriverIO** sends session creation request with `tauri:options` capability
7. **tauri-driver** sees `tauri:options.application` path
8. **tauri-driver** launches `agentmux.exe`
9. **tauri-driver** connects msedgedriver to the app's WebView
10. **Tests run** - WebdriverIO sends commands, tauri-driver executes them
11. **`afterSession` hook** kills tauri-driver process
12. **tauri-driver cleanup** closes app and msedgedriver

---

## Part 4: Tauri's Official Best Practices

### Official Configuration (from Tauri v2 docs)

#### Directory Structure

```
your-tauri-app/
├── src-tauri/
│   └── target/
│       ├── debug/
│       │   └── your-app
│       └── release/
│           └── your-app
└── e2e-tests/              ← Create this
    ├── package.json
    ├── wdio.conf.js
    └── test/
        └── specs/
            └── example.e2e.js
```

#### package.json (Recommended)

```json
{
  "name": "webdriverio",
  "version": "1.0.0",
  "scripts": {
    "test": "wdio run wdio.conf.js"
  },
  "devDependencies": {
    "@wdio/cli": "^9.19.0",
    "@wdio/local-runner": "^9.19.0",
    "@wdio/mocha-framework": "^9.19.0",
    "@wdio/spec-reporter": "^9.19.0",
    "webdriverio": "^9.19.0"
  }
}
```

#### wdio.conf.js (Official Template)

```javascript
import path from 'path';
import { fileURLToPath } from 'url';
import { spawn, spawnSync } from 'child_process';
import os from 'os';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Track tauri-driver process
let tauriDriver;

export const config = {
  // ====================
  // Runner Configuration
  // ====================
  runner: 'local',

  // ==================
  // Specify Test Files
  // ==================
  specs: ['./test/specs/**/*.js'],
  exclude: [],

  // ============
  // Capabilities
  // ============
  maxInstances: 1, // Run tests sequentially

  capabilities: [
    {
      maxInstances: 1,
      // 🔑 KEY: Use tauri:options, NOT browserName
      'tauri:options': {
        // Path to your Tauri binary
        application: '../src-tauri/target/release/your-app-name',
      },
    },
  ],

  // ===================
  // Test Configurations
  // ===================
  logLevel: 'info',
  bail: 0,
  waitforTimeout: 30000,
  connectionRetryTimeout: 120000,
  connectionRetryCount: 3,

  // WebDriver settings
  hostname: '127.0.0.1',
  port: 4444, // tauri-driver default port

  // NO services array needed
  services: [],

  framework: 'mocha',
  reporters: ['spec'],

  // =====
  // Hooks
  // =====

  /**
   * Build the Tauri app before tests
   */
  onPrepare: function () {
    // Build in release mode
    return spawnSync('cargo', ['build', '--release'], {
      cwd: path.join(__dirname, '..', 'src-tauri'),
      stdio: 'inherit',
    });
  },

  /**
   * Start tauri-driver before creating session
   */
  beforeSession: function () {
    // Path to tauri-driver (installed via cargo)
    const tauriDriverPath = path.resolve(
      os.homedir(),
      '.cargo',
      'bin',
      'tauri-driver'
    );

    // Spawn tauri-driver
    tauriDriver = spawn(tauriDriverPath, [], {
      stdio: [null, process.stdout, process.stderr],
    });

    console.log('[tauri-driver] Started');
  },

  /**
   * Stop tauri-driver after session ends
   */
  afterSession: function () {
    if (tauriDriver) {
      tauriDriver.kill();
      console.log('[tauri-driver] Stopped');
    }
  },

  // =============
  // Mocha Options
  // =============
  mochaOpts: {
    ui: 'bdd',
    timeout: 60000, // 1 minute
  },
};
```

#### Test Spec Example

```javascript
describe('Hello Tauri', () => {
  it('should be cordial', async () => {
    const header = await $('body > h1');
    const text = await header.getText();
    expect(text).toMatch(/^[hH]ello/);
  });

  it('should be excited', async () => {
    const header = await $('body > h1');
    const text = await header.getText();
    expect(text).toMatch(/!$/);
  });

  it('should be easy on the eyes', async () => {
    const body = await $('body');
    const bgColor = await body.getCSSProperty('background-color');
    expect(bgColor.parsed.hex).toBe('#87ceeb');
  });
});
```

### Running Tests

```bash
cd e2e-tests
npm install
npm test
```

### Expected Output

```
[tauri-driver] Started

Execution of 1 workers started at 2023-11-15T10:30:00.000Z

[0-0] RUNNING in undefined - /test/specs/example.e2e.js
[0-0] PASSED in undefined - /test/specs/example.e2e.js

 "spec" Reporter:
------------------------------------------------------------------
[chrome (linux) #0-0] Running: chrome (v108.0.5359.94) on linux
[chrome (linux) #0-0] Session ID: 12345
[chrome (linux) #0-0]
[chrome (linux) #0-0] » /test/specs/example.e2e.js
[chrome (linux) #0-0] Hello Tauri
[chrome (linux) #0-0]    ✓ should be cordial
[chrome (linux) #0-0]    ✓ should be excited
[chrome (linux) #0-0]    ✓ should be easy on the eyes
[chrome (linux) #0-0]
[chrome (linux) #0-0] 3 passing (2s)

Spec Files:      1 passed, 1 total (100% completed) in 00:00:05

[tauri-driver] Stopped
```

---

## Part 5: What We Did Wrong

### Our Configuration Issues

#### Issue 1: Wrong Capability Format

**What We Used:**
```javascript
capabilities: [{
  browserName: 'msedge',
  'ms:edgeOptions': {
    binary: path.join(__dirname, 'src-tauri', 'target', 'release', 'agentmux.exe'),
    args: [],
  },
}]
```

**Why It's Wrong:**
- `browserName: 'msedge'` tells WebdriverIO to treat this as a Microsoft Edge browser
- `ms:edgeOptions` is for launching Edge browser, not Tauri apps
- This causes msedgedriver to try launching `agentmux.exe` as Edge (which fails)

**What We Should Use:**
```javascript
capabilities: [{
  maxInstances: 1,
  'tauri:options': {
    application: path.join(__dirname, 'src-tauri', 'target', 'release', 'agentmux.exe'),
  },
}]
```

**Why It's Right:**
- `tauri:options` is recognized by tauri-driver
- tauri-driver knows how to launch Tauri apps properly
- No `browserName` needed - tauri-driver handles that

#### Issue 2: Starting tauri-driver in Wrong Hook

**What We Used:**
```javascript
onPrepare: async function () {
  const tauriDriver = spawn('tauri-driver', [
    '--native-driver',
    path.join(__dirname, 'msedgedriver.exe')
  ], {
    stdio: 'inherit',
  });

  global.tauriDriverProcess = tauriDriver;
  await new Promise(resolve => setTimeout(resolve, 2000));
}
```

**Why It's Problematic:**
- `onPrepare` runs before ALL workers start (once per test run)
- This works, but official docs use `beforeSession`
- `beforeSession` runs before each session (better lifecycle management)

**What Official Docs Use:**
```javascript
beforeSession: function () {
  tauriDriver = spawn(tauriDriverPath, [], {
    stdio: [null, process.stdout, process.stderr],
  });
}
```

#### Issue 3: Passing msedgedriver Path

**What We Did:**
```javascript
spawn('tauri-driver', [
  '--native-driver',
  path.join(__dirname, 'msedgedriver.exe')
])
```

**Official Approach:**
```javascript
spawn(tauriDriverPath, [])
```

**Why:**
- Official docs assume msedgedriver is in PATH
- If not in PATH, you can pass `--native-driver` flag
- But typically, just having it in the same directory works

#### Issue 4: Port Configuration

**What We Did:**
```javascript
automationProtocol: 'webdriver',
port: 4444,
services: [],
```

**Official Docs:**
```javascript
hostname: '127.0.0.1',
port: 4444,
// NO automationProtocol specified
// NO services array
```

### The Root Cause

We were trying to use **msedgedriver directly** instead of letting **tauri-driver manage msedgedriver**.

**Wrong Flow:**
```
WebdriverIO → msedgedriver → agentmux.exe (FAILS)
```

**Correct Flow:**
```
WebdriverIO → tauri-driver → msedgedriver → agentmux.exe (WORKS)
```

---

## Part 6: Platform Requirements & Compatibility

### Windows (Our Platform)

**Requirements:**
- **msedgedriver.exe** - Version MUST match Edge/WebView2 version
- **tauri-driver** - Installed via `cargo install tauri-driver`
- **WebView2** - Built into Windows 11, or installed separately on Windows 10

**Checking Edge Version:**
```powershell
(Get-AppxPackage -Name "Microsoft.Edge").Version
# Output: 140.0.3485.54
```

**Downloading msedgedriver:**
- Official site: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/
- Download matching version (e.g., 140.0.3485.54)
- Extract `msedgedriver.exe`
- Add to PATH or specify with `--native-driver`

**Version Mismatch Tolerance:**
- Minor version differences (e.g., 140.0.3485.54 vs 141.0.3537.71) may work
- Major version differences will cause issues
- Best practice: Match exactly

### Linux

**Requirements:**
- **WebKitWebDriver** - Usually `webkit2gtk-driver` package
- **tauri-driver** - Installed via `cargo install tauri-driver`

**Installation (Debian/Ubuntu):**
```bash
sudo apt-get install webkit2gtk-driver
```

**Check if available:**
```bash
which WebKitWebDriver
```

### macOS

**Status:** NOT SUPPORTED

**Reason:** No WKWebView driver tool available from Apple

**Alternatives:**
- iOS testing via Appium 2 (not streamlined yet)
- Manual testing only

---

## Part 7: Alternative Testing Approaches

### Option 1: WebdriverIO + tauri-driver (RECOMMENDED)

**Pros:**
- ✅ Official Tauri approach
- ✅ Full E2E testing support
- ✅ Cross-platform (Windows/Linux)
- ✅ Standard WebDriver protocol
- ✅ Rich ecosystem (reporters, plugins)
- ✅ Well-documented

**Cons:**
- ❌ Windows/Linux only (no macOS)
- ❌ Requires cargo toolchain for tauri-driver
- ❌ msedgedriver version matching

**When to Use:**
- Full E2E UI testing
- Testing user interactions
- Visual regression testing
- Integration testing with real WebView

### Option 2: Selenium + tauri-driver

**Same as WebdriverIO, but:**
- Uses Selenium WebDriver instead of WebdriverIO
- More verbose API
- Java/Python/C# clients available
- Older, more mature ecosystem

**Official Tauri Example:**
https://v2.tauri.app/develop/tests/webdriver/example/selenium/

**Configuration:**
```javascript
const { Builder } = require('selenium-webdriver');
const os = require('os');
const path = require('path');
const { spawn, spawnSync } = require('child_process');

// Build app
spawnSync('cargo', ['build', '--release']);

// Start tauri-driver
const tauriDriver = spawn(
  path.resolve(os.homedir(), '.cargo', 'bin', 'tauri-driver'),
  [],
  { stdio: [null, process.stdout, process.stderr] }
);

// Create session
const capabilities = {
  'tauri:options': {
    application: '../../target/release/app-name',
  },
};

const driver = await new Builder()
  .withCapabilities(capabilities)
  .usingServer('http://127.0.0.1:4444/')
  .build();

// Run tests
const element = await driver.findElement(By.css('button'));
await element.click();

// Cleanup
await driver.quit();
tauriDriver.kill();
```

### Option 3: Rust-based Testing (Advanced)

**Approach:** Write tests directly in Rust using `tauri::test` module

**Pros:**
- ✅ No external dependencies
- ✅ Type-safe
- ✅ Fast
- ✅ Test Tauri commands directly

**Cons:**
- ❌ No real WebView testing (uses mock runtime)
- ❌ Can't test UI interactions
- ❌ Limited to backend logic

**When to Use:**
- Unit testing Tauri commands
- Testing IPC layer
- Backend logic testing

**Example:**
```rust
#[cfg(test)]
mod tests {
  use tauri::test::{mock_builder, mock_context, MockRuntime};

  #[test]
  fn test_command() {
    let app = mock_builder()
      .invoke_handler(tauri::generate_handler![my_command])
      .build(mock_context())
      .expect("failed to build app");

    let result = tauri::test::get_ipc_response(
      &app,
      tauri::webview::InvokeRequest {
        cmd: "my_command".into(),
        args: serde_json::json!({"arg": "value"}),
      },
    );

    assert!(result.is_ok());
  }
}
```

### Option 4: Playwright (NOT RECOMMENDED for Tauri)

**Why We Tried It:**
- Familiar API
- Good documentation
- Works for Electron and standalone WebView2 apps

**Why It Failed:**
- Relies on Chrome DevTools Protocol (CDP)
- Requires `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`
- Tauri's WebView2 integration doesn't respect this environment variable
- Only works for standalone WebView2 apps, not Tauri

**Verdict:** ❌ Don't use Playwright for Tauri

---

## Part 8: Official Resources

### Documentation

1. **Main WebDriver Guide:**
   - https://v2.tauri.app/develop/tests/webdriver/

2. **WebdriverIO Example:**
   - https://v2.tauri.app/develop/tests/webdriver/example/webdriverio/

3. **Selenium Example:**
   - https://v2.tauri.app/develop/tests/webdriver/example/selenium/

4. **CI/CD Integration:**
   - https://v2.tauri.app/develop/tests/webdriver/ci/

5. **Tauri Testing Overview:**
   - https://v2.tauri.app/develop/tests/

### Example Repositories

1. **Official WebDriver Example:**
   - https://github.com/tauri-apps/webdriver-example
   - Contains working examples for both v1 and v2
   - Uses pnpm for package management

2. **Smoke Tests:**
   - https://github.com/tauri-apps/smoke-tests
   - Collection of framework examples
   - Used by Tauri team for testing

### Community Resources

1. **GitHub Discussions:**
   - https://github.com/tauri-apps/tauri/discussions/10123
   - "Is it possible to e2e test a Tauri app?"

2. **Stack Overflow:**
   - Tag: `tauri`
   - Search: "tauri webdriver"

---

## Part 9: Recommended Next Steps

### Immediate Actions

1. **Fix wdio.conf.js** - Replace `browserName` with `tauri:options`

2. **Update capabilities:**
   ```javascript
   capabilities: [{
     maxInstances: 1,
     'tauri:options': {
       application: path.join(__dirname, 'src-tauri', 'target', 'release', 'agentmux.exe'),
     },
   }]
   ```

3. **Remove unnecessary config:**
   - Remove `automationProtocol: 'webdriver'`
   - Keep `port: 4444`
   - Keep `services: []`

4. **Verify msedgedriver in PATH** or update tauri-driver spawn to include `--native-driver` flag

5. **Run tests again**

### Testing the Fix

```bash
# Ensure release build exists
cd D:\Code\WebProjects\agentmux\apps\desktop
npm run tauri:build

# Verify binary exists
ls src-tauri/target/release/agentmux.exe

# Run tests
npm run test:e2e
```

### Expected Outcome

**If successful:**
```
[wdio] Starting tauri-driver server...
[wdio] ✓ tauri-driver server started

Execution of 1 workers started...

[0-0] RUNNING in undefined - ./tests/e2e/claude-terminal-interaction.spec.js
[0-0] PASSED in undefined - ./tests/e2e/claude-terminal-interaction.spec.js

Spec Files:      1 passed, 1 total
Tests:           4 passed, 4 total
Duration:        ~2 minutes

[wdio] Stopping tauri-driver server...
[wdio] ✓ tauri-driver server stopped
```

**If still fails:**
- Check tauri-driver output for errors
- Verify msedgedriver version matches Edge version
- Check if agentmux.exe launches manually
- Enable trace logging: `logLevel: 'trace'` in wdio.conf.js

### Long-term Improvements

1. **Add CI/CD integration** - Run tests on GitHub Actions

2. **Expand test coverage** - Add more test specs

3. **Screenshot/video capture** - For debugging failures

4. **Parallel testing** - Once stable (increase maxInstances)

5. **Visual regression testing** - Using wdio-image-comparison-service

---

## Part 10: Key Takeaways

### Critical Understanding

1. **tauri-driver is NOT a test framework** - It's a WebDriver server
2. **WebdriverIO is NOT a WebDriver server** - It's a test client/framework
3. **They work together** - Client (WebdriverIO) → Server (tauri-driver) → Native Driver (msedgedriver) → App (agentmux.exe)
4. **Use `tauri:options` capability** - This is how you tell tauri-driver which app to launch
5. **Don't use `browserName`** - That's for browsers, not Tauri apps
6. **Trust official docs** - When a framework has official testing tools, use them

### What We Learned

1. **Playwright doesn't work for Tauri** - Despite working for WebView2 standalone apps
2. **Version matching matters** - msedgedriver must match Edge/WebView2 version (or be close)
3. **tauri-driver manages msedgedriver** - Don't try to use msedgedriver directly
4. **Official examples exist** - We should have referenced them from the start
5. **Architecture matters** - Understanding the client-server model is crucial

### Decision Framework

**When choosing a testing approach for Tauri:**

1. ✅ **Use tauri-driver + WebdriverIO/Selenium** for E2E UI testing
2. ✅ **Use Rust `tauri::test`** for backend/command unit tests
3. ✅ **Use `@tauri-apps/api/mocks`** for frontend unit tests
4. ❌ **Don't use Playwright** - Doesn't work with Tauri
5. ❌ **Don't use Puppeteer** - Same issue as Playwright (CDP-based)

---

## Conclusion

Our issue was **architectural misunderstanding**, not a technical limitation. We were trying to use msedgedriver directly (browser automation) when we should have been using tauri-driver (Tauri app automation).

**The fix is simple:** Change our `wdio.conf.js` capabilities from `browserName: 'msedge'` to `'tauri:options': { application: '...' }`.

This allows tauri-driver to properly launch our Tauri app instead of msedgedriver trying to launch it as Edge browser.

**Official Tauri documentation has working examples** that we should follow exactly. The webdriver-example repository on GitHub demonstrates the correct approach.

---

## References

- Tauri WebDriver Docs: https://v2.tauri.app/develop/tests/webdriver/
- WebdriverIO Example: https://v2.tauri.app/develop/tests/webdriver/example/webdriverio/
- Selenium Example: https://v2.tauri.app/develop/tests/webdriver/example/selenium/
- Official Example Repo: https://github.com/tauri-apps/webdriver-example
- tauri-driver on crates.io: https://crates.io/crates/tauri-driver
- WebdriverIO Docs: https://webdriver.io/docs/gettingstarted
- Microsoft Edge WebDriver: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/
