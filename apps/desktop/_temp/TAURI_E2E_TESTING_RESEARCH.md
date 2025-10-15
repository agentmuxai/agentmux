# Tauri E2E Testing Research Report
**Date:** 2025-10-15
**Focus:** Playwright + Tauri WebView2 on Windows

## Executive Summary

✅ **Playwright CAN work with Tauri** - The issue was in our test setup, not a fundamental limitation.

**Root Cause:** Our tests were using a fixed port (9222) and not managing WebView2 user data directories, causing port conflicts and connection failures.

**Solution:** Use dynamic ports and unique user data folders per test, as documented in Playwright's official WebView2 guide.

---

## Current Problem Analysis

### What We Did Wrong

1. **Fixed Port Usage**
   ```typescript
   // ❌ WRONG - All tests fight for port 9222
   const debugPort = 9222;
   ```

2. **No User Data Isolation**
   - WebView2 creates lock files in its user data directory
   - Multiple instances can't share the same data directory
   - Tests were interfering with each other

3. **Single-Instance App Logic**
   - Our app has IPC-based single-instance detection
   - New instances were connecting to existing instance instead of launching fresh

### Why Tests Failed

```
[Tauri E2E] Waiting for debugging port to be ready...
Error: Timeout waiting for port 9222 to be ready after 60000ms
```

**Diagnosis:**
- Port 9222 was never opened because `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` wasn't being respected
- WebView2 needs both the environment variable AND a unique user data folder
- Node.js `spawn()` environment inheritance wasn't working correctly

---

## Solution: Playwright WebView2 Best Practices

### Official Playwright Approach

From https://playwright.dev/docs/webview2:

```typescript
import { test, expect } from '@playwright/test';
import { chromium } from 'playwright';
import { spawn, ChildProcess } from 'child_process';

// Key Insight: Use DYNAMIC ports and unique data directories
let webview2Process: ChildProcess;
let debugPort: number;

test.beforeEach(async () => {
  // 1. Generate unique port and data directory
  debugPort = 9000 + Math.floor(Math.random() * 1000);
  const userDataDir = `./test-data/webview2-${Date.now()}`;

  // 2. Set environment variables
  webview2Process = spawn('path/to/app.exe', [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
    },
  });

  // 3. Wait for port to be ready
  await waitForPort(debugPort);
});

test.afterEach(async () => {
  // Cleanup
  webview2Process.kill();
});
```

### Key Requirements

1. **Dynamic Port Assignment**
   - Generate random port per test (9000-9999 range)
   - Avoids conflicts between parallel tests

2. **Unique User Data Directories**
   - Each test gets its own `WEBVIEW2_USER_DATA_FOLDER`
   - Prevents lock file conflicts
   - Format: `./test-data/webview2-${timestamp}`

3. **Environment Variable Passing**
   - Must set BOTH:
     - `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
     - `WEBVIEW2_USER_DATA_FOLDER`

4. **Connection Method**
   ```typescript
   const browser = await chromium.connectOverCDP(`http://localhost:${debugPort}`);
   const context = browser.contexts()[0];
   const page = context.pages()[0];
   ```

---

## Implementation Plan

### Phase 1: Fix Test Infrastructure (1-2 hours)

**Update `tests/e2e/helpers/tauri-app.ts`:**

```typescript
export async function launchTauriApp(options?: {
  executablePath?: string;
  timeout?: number;
}): Promise<TauriAppInstance> {
  // Generate unique port and data directory
  const debugPort = 9000 + Math.floor(Math.random() * 1000);
  const userDataDir = path.join(process.cwd(), 'test-data', `webview2-${Date.now()}`);

  console.log(`[Tauri E2E] Debug port: ${debugPort}`);
  console.log(`[Tauri E2E] User data: ${userDataDir}`);

  const tauriProcess = spawn(executablePath, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
      // Disable single-instance check for tests
      AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
    },
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  // Wait for port
  await waitForPort(debugPort, timeout);

  // Connect via CDP
  const browser = await chromium.connectOverCDP(`http://localhost:${debugPort}`, { timeout });
  const context = browser.contexts()[0];
  const page = context.pages()[0];

  return { browser, page, process: tauriProcess, debugPort, userDataDir };
}
```

**Update cleanup:**

```typescript
export async function closeTauriApp(instance: TauriAppInstance): Promise<void> {
  await instance.browser.close();
  instance.process.kill();

  // Cleanup user data directory
  try {
    await fs.rm(instance.userDataDir, { recursive: true, force: true });
  } catch (err) {
    console.warn(`Failed to cleanup ${instance.userDataDir}`);
  }
}
```

### Phase 2: Add Single-Instance Bypass (30 mins)

**Update `src-tauri/src/main.rs`:**

```rust
fn main() {
    // Check for test mode
    let disable_single_instance = std::env::var("AGENTMUX_DISABLE_SINGLE_INSTANCE").is_ok();

    if !disable_single_instance {
        // Existing single-instance check
        if let Err(e) = check_single_instance() {
            eprintln!("{}", e);
            return;
        }
    }

    // Rest of main()...
}
```

### Phase 3: Update Test Configuration (15 mins)

**Ensure `playwright-e2e.config.ts` has:**

```typescript
export default defineConfig({
  testDir: './tests/e2e',
  workers: 1, // Sequential execution
  timeout: 120000,

  use: {
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },

  // No webServer - tests launch app directly
});
```

### Phase 4: Run Tests (5 mins)

```bash
cd apps/desktop
npx playwright test tests/e2e/claude-terminal-interaction.spec.ts --config=playwright-e2e.config.ts --headed
```

---

## Alternative Approaches Considered

### Option 1: tauri-driver + WebdriverIO
**Pros:**
- Official Tauri recommendation
- Works on Linux and Windows
- Integrated with Tauri ecosystem

**Cons:**
- Requires `cargo install tauri-driver`
- Requires msedgedriver.exe (version matching)
- More complex setup
- Less familiar API than Playwright

**Verdict:** ❌ Rejected - Playwright is simpler and we already have infrastructure

### Option 2: Playwright with tauri-driver Bridge
**Pros:**
- Combines Playwright API with Tauri's driver
- Best of both worlds

**Cons:**
- Requires both tools
- Additional complexity
- Not well documented

**Verdict:** ❌ Rejected - Overkill for our use case

### Option 3: Manual Testing Only
**Pros:**
- No automation infrastructure needed
- Simple

**Cons:**
- Not scalable
- Regression-prone
- User said "getting e2e tests is critical"

**Verdict:** ❌ Rejected - User requirement

### Option 4: Pure Playwright with Dynamic Ports (CHOSEN)
**Pros:**
- Works with WebView2 directly
- Official Playwright support
- Simple, familiar API
- Already have most infrastructure
- Just needs fixes, not rewrite

**Cons:**
- Windows-only (but that's our target)
- Requires unique data directories

**Verdict:** ✅ **RECOMMENDED** - Best balance of simplicity and functionality

---

## Expected Outcomes

### After Implementation

**Tests Should:**
1. Launch Tauri app with unique CDP port (e.g., 9347)
2. Create isolated user data directory
3. Connect Playwright to WebView2 via CDP
4. Execute test interactions (clicks, keyboard)
5. Verify UI state and behavior
6. Cleanup app and data directory

**Test Output:**
```
Running 4 tests using 1 worker

[Tauri E2E] Launching Tauri app from: ...agentmux.exe
[Tauri E2E] Debug port: 9347
[Tauri E2E] User data: ./test-data/webview2-1729012345
[Tauri E2E] Waiting for debugging port to be ready...
[Tauri E2E] ✓ Debugging port ready
[Tauri E2E] Connecting Playwright to debugging port...
[Tauri E2E] ✓ Playwright connected
[Tauri E2E] ✓ Connected to page: tauri://localhost

  ✓ TC1: Click terminal output → input focused (2.3s)
  ✓ TC2: Arrow keys navigate without scrolling (1.8s)
  ✓ TC3: Claude responds to Enter key (5.2s)
  ✓ TC4: Input and output appear continuous (0.9s)

4 passed (10.2s)
```

---

## Implementation Checklist

- [ ] Update `tauri-app.ts` with dynamic ports
- [ ] Add unique user data directory per test
- [ ] Implement cleanup in `closeTauriApp()`
- [ ] Add `AGENTMUX_DISABLE_SINGLE_INSTANCE` env var support in Rust
- [ ] Update `.gitignore` to exclude `test-data/`
- [ ] Run tests to verify connection works
- [ ] Add cleanup script for stale test data
- [ ] Document test execution in README

---

## References

1. **Playwright WebView2 Guide**: https://playwright.dev/docs/webview2
2. **Tauri WebDriver Docs**: https://v2.tauri.app/develop/tests/webdriver/
3. **Tauri Windows E2E Demo**: https://github.com/rzmk/tauri-windows-e2e-demo
4. **Microsoft Edge WebDriver**: https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/webdriver
5. **WebView2 Remote Debugging**: https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/remote-debugging-desktop

---

## Conclusion

**The solution is straightforward:** Use Playwright's official WebView2 approach with dynamic ports and unique user data directories. Our existing infrastructure is 90% correct - it just needs these key fixes to work reliably.

**Estimated Time to Working Tests:** 2-3 hours

**Next Action:** Implement Phase 1 (update `tauri-app.ts`) and test immediately.
