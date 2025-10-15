# E2E Tests for AgentMux Desktop

## Overview

End-to-end tests using Playwright to test the actual Tauri desktop application.

**What's tested:**
- ✅ Launching the Tauri app
- ✅ Spawning an agent
- ✅ Sending messages via UI
- ✅ Receiving responses from Claude
- ✅ Multiple sequential messages
- ✅ Console log verification

---

## Prerequisites

### 1. Build the Application First

**Important:** E2E tests require a built executable.

```bash
# Build the app
npm run tauri:build

# This creates:
# - apps/desktop/src-tauri/target/release/agentmux.exe (portable)
# - apps/desktop/src-tauri/target/release/bundle/msi/AgentMux Desktop_*.msi (installer)
```

### 2. Install Playwright Dependencies

```bash
# Install Playwright browsers (if not already installed)
npx playwright install chromium
```

---

## Running Tests

### Run All E2E Tests

```bash
npm run test:playwright
```

### Run Tests in UI Mode (Interactive)

```bash
npm run test:playwright:ui
```

This opens Playwright's interactive UI where you can:
- Step through tests
- View screenshots
- Inspect DOM
- See console logs

### Run Tests in Debug Mode

```bash
npm run test:playwright:debug
```

This runs tests with a debugger attached.

### View Test Report

```bash
npm run test:playwright:report
```

---

## How It Works

### Architecture

```
┌──────────────────────┐
│  Playwright Test     │
│  (Node.js)           │
└──────────┬───────────┘
           │
           │ 1. Launch process
           ↓
┌──────────────────────┐
│  Tauri App           │  Set env var:
│  (agentmux.exe)      │  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
│                      │    --remote-debugging-port=9222
└──────────┬───────────┘
           │
           │ 2. WebView2 opens debugging port
           ↓
┌──────────────────────┐
│  WebView2            │  Listening on:
│  Remote Debugging    │  http://localhost:9222
│  (CDP Protocol)      │
└──────────┬───────────┘
           │
           │ 3. Playwright connects via CDP
           ↓
┌──────────────────────┐
│  Playwright          │
│  Browser Connection  │  Can now:
│                      │  - Inspect elements
│                      │  - Click buttons
│                      │  - Type text
│                      │  - Read console logs
└──────────────────────┘
```

### Key Components

#### 1. Tauri App Helper (`helpers/tauri-app.ts`)

Provides utilities for:
- **`launchTauriApp()`** - Launches Tauri with WebView2 debugging enabled
- **`closeTauriApp()`** - Gracefully shuts down the app
- **`takeDebugScreenshot()`** - Captures screenshots for debugging

#### 2. Test Spec (`agent-communication.spec.ts`)

Contains the actual E2E tests:

**Test 1: Complete Communication Flow**
1. Launch app
2. Click "Spawn Agent"
3. Type message in input
4. Press Enter (or click Send)
5. Wait for response
6. Verify response appears

**Test 2: Multiple Sequential Messages**
- Sends 3 messages one after another
- Verifies all are processed

**Test 3: Console Log Verification**
- Captures browser console logs
- Verifies WebSocket logs are being generated

---

## Test Configuration

### Playwright Config (`playwright.config.ts`)

Key settings:
```typescript
{
  testDir: './tests/e2e',
  timeout: 60000,              // 60s per test
  fullyParallel: false,         // One app instance at a time
  retries: 2,                   // Retry failed tests on CI
  use: {
    trace: 'on-first-retry',    // Capture trace on failure
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  }
}
```

---

## Debugging Failed Tests

### 1. View Screenshots

Screenshots are saved on failure:
```
test-results/screenshots/
├── 01-app-loaded-1234567890.png
├── 02-spawn-button-not-found-1234567890.png
├── 03-agent-spawned-1234567890.png
└── ...
```

### 2. View Trace Files

Trace files (if enabled) contain:
- Timeline of actions
- Network requests
- Console logs
- DOM snapshots

View with:
```bash
npx playwright show-trace test-results/trace.zip
```

### 3. Run in Headed Mode

See the browser window during testing:
```bash
npx playwright test --headed
```

### 4. Use UI Mode

Interactive debugging:
```bash
npm run test:playwright:ui
```

### 5. Check Console Output

The test helper logs everything:
```
[Tauri E2E] Launching Tauri app from: ...
[Tauri stdout] WebSocket server listening on 127.0.0.1:9999
[Test] ✓ App loaded
[Test] ✓ Agent spawned
[Test] ✓ Message typed
[Browser Console] [WS:127.0.0.1:xxxxx] ← Received text message #1: ...
```

---

## Common Issues

### Issue: "Timeout waiting for port 9222"

**Cause:** App took too long to start or didn't enable debugging port

**Solutions:**
- Increase timeout in `launchTauriApp({ timeout: 90000 })`
- Check app builds correctly: `npm run tauri:build`
- Verify WebView2 is installed on Windows

### Issue: "Could not find Spawn Agent button"

**Cause:** UI changed or button selector is wrong

**Solutions:**
- View screenshot: `test-results/screenshots/02-spawn-button-not-found-*.png`
- Check console output for available buttons
- Update selector in test spec

### Issue: "No response received within 30 seconds"

**Cause:** Claude not responding or response not detected

**Solutions:**
- Check if Claude CLI is installed and in PATH
- View screenshot: `test-results/screenshots/07-response-received-*.png`
- Increase timeout in test
- Check console logs for WebSocket errors

### Issue: "Browser connection failed"

**Cause:** WebView2 debugging port not accessible

**Solutions:**
- Ensure `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` is set correctly
- Check if port 9222 is already in use
- Try a different port: `launchTauriApp({ debugPort: 9223 })`

---

## CI/CD Integration

### GitHub Actions Example

```yaml
name: E2E Tests

on: [push, pull_request]

jobs:
  e2e:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v3

      - name: Setup Node.js
        uses: actions/setup-node@v3
        with:
          node-version: '20'

      - name: Install dependencies
        run: npm ci

      - name: Build Tauri app
        run: npm run tauri:build

      - name: Install Playwright
        run: npx playwright install --with-deps chromium

      - name: Run E2E tests
        run: npm run test:playwright

      - name: Upload test results
        if: always()
        uses: actions/upload-artifact@v3
        with:
          name: playwright-report
          path: playwright-report/
```

---

## Extending Tests

### Add New Test

```typescript
test('should do something', async () => {
  if (!tauriApp) throw new Error('Tauri app not initialized');
  const { page } = tauriApp;

  // Your test logic here
  await page.click('button');
  await expect(page.locator('.result')).toBeVisible();
});
```

### Add New Selector Strategy

```typescript
const customSelectors = [
  'button[data-testid="my-button"]',
  'button:has-text("My Button")',
  '[aria-label="My Button"]',
];

let button = null;
for (const selector of customSelectors) {
  try {
    button = page.locator(selector).first();
    if (await button.isVisible({ timeout: 2000 })) {
      break;
    }
  } catch (e) {
    // Try next selector
  }
}
```

### Test Against Different Versions

```typescript
test('v0.3.1 features', async () => {
  tauriApp = await launchTauriApp({
    executablePath: '../../releases/v0.3.1/agentmux-desktop-v0.3.1-portable.exe',
  });

  // Test v0.3.1 specific features
});
```

---

## Best Practices

1. **Always build before testing**
   ```bash
   npm run tauri:build && npm run test:playwright
   ```

2. **Use descriptive test names**
   ```typescript
   test('should spawn agent and send message successfully', async () => {
   ```

3. **Take screenshots at key points**
   ```typescript
   await takeDebugScreenshot(page, 'after-clicking-spawn');
   ```

4. **Log progress**
   ```typescript
   console.log('[Test] Step 1: Clicking button...');
   ```

5. **Handle timing issues with waits**
   ```typescript
   await page.waitForTimeout(2000); // Wait for animation
   await page.waitForSelector('button', { state: 'visible' });
   ```

6. **Use multiple selector strategies**
   - Fallback if UI changes
   - More robust tests

---

## Performance

**Typical test run:**
- Launch app: ~10-20 seconds
- Per test: ~5-15 seconds
- Full suite: ~1-2 minutes

**Optimization tips:**
- Reuse app instance across tests (done in `beforeAll`)
- Use `fullyParallel: false` (app can't run multiple instances)
- Skip unnecessary waits

---

## Resources

- [Playwright Documentation](https://playwright.dev/docs/intro)
- [Tauri Testing Guide](https://v2.tauri.app/develop/tests/)
- [WebView2 Remote Debugging](https://learn.microsoft.com/en-us/microsoft-edge/webview2/)
- [CDP Protocol](https://chromedevtools.github.io/devtools-protocol/)

---

**Status:** ✅ Ready to use
**Last Updated:** 2025-10-14
