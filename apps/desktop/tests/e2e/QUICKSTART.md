# Quick Start: E2E Testing

## 5-Minute Setup

### 1. Build the App (Required)

```bash
cd apps/desktop
npm run tauri:build
```

**Wait for:** `Finished 1 bundle at: ...AgentMux Desktop_0.3.1_x64_en-US.msi`

---

### 2. Run the E2E Test

```bash
npm run test:playwright
```

**Expected output:**
```
[Tauri E2E] Launching Tauri app from: src-tauri/target/release/agentmux.exe
[Tauri E2E] WebView2 debugging port: 9222
[Tauri E2E] Waiting for debugging port to be ready...
[Tauri stdout] WebSocket server listening on 127.0.0.1:9999
[Tauri E2E] ✓ Debugging port ready
[Tauri E2E] ✓ Playwright connected
[Test] ✓ Tauri app launched successfully
[Test] Step 1: Waiting for app to load...
[Test] ✓ App loaded
[Test] Step 2: Looking for "Spawn Agent" button...
[Test] ✓ Found button with selector: button:has-text("Spawn Agent")
[Test] Clicking "Spawn Agent" button...
[Test] ✓ Agent spawned
[Test] Step 3: Looking for message input field...
[Test] ✓ Found input with selector: input[type="text"]
[Test] Step 4: Typing message: "Hello, this is an automated test message!"
[Test] ✓ Message typed
[Test] Step 5: Sending message...
[Test] ✓ Pressed Enter
[Test] Step 6: Waiting for response from Claude...
[Test] ✓ Response detected!
[Test] ✓ Test completed successfully!

✓ 3 tests passed (1.5m)
```

---

### 3. View Results

**HTML Report:**
```bash
npm run test:playwright:report
```

Opens browser with:
- Test results
- Screenshots
- Timing information
- Error details (if any)

---

## What the Test Does

```
┌─────────────────────────────────────┐
│  1. Launch Tauri app                │
│     (with WebView2 debugging)       │
└─────────────────┬───────────────────┘
                  │
                  ↓
┌─────────────────────────────────────┐
│  2. Connect Playwright to app       │
│     (via CDP on port 9222)          │
└─────────────────┬───────────────────┘
                  │
                  ↓
┌─────────────────────────────────────┐
│  3. Click "Spawn Agent" button      │
└─────────────────┬───────────────────┘
                  │
                  ↓
┌─────────────────────────────────────┐
│  4. Type message in input field     │
│     "Hello, this is a test!"        │
└─────────────────┬───────────────────┘
                  │
                  ↓
┌─────────────────────────────────────┐
│  5. Press Enter (or click Send)     │
└─────────────────┬───────────────────┘
                  │
                  ↓
┌─────────────────────────────────────┐
│  6. Wait for Claude response        │
│     (max 30 seconds)                │
└─────────────────┬───────────────────┘
                  │
                  ↓
┌─────────────────────────────────────┐
│  7. Verify response appears         │
│     ✓ Test passes                   │
└─────────────────────────────────────┘
```

---

## Interactive Mode (Recommended for Debugging)

```bash
npm run test:playwright:ui
```

**Benefits:**
- ✅ Step through test line-by-line
- ✅ See screenshots at each step
- ✅ Inspect DOM in real-time
- ✅ View console logs
- ✅ Re-run individual tests

**Perfect for:**
- Understanding how tests work
- Debugging failures
- Developing new tests

---

## Debug Mode (Pause on Errors)

```bash
npm run test:playwright:debug
```

Automatically pauses when test fails, allowing you to:
- Inspect page state
- Try different selectors
- Check console errors

---

## Common Issues

### ❌ "Timeout waiting for port 9222"

**Fix:** App didn't start. Rebuild it:
```bash
npm run tauri:build
```

### ❌ "Could not find Spawn Agent button"

**Fix:** View screenshot to see actual UI:
```bash
ls test-results/screenshots/
# Open: 02-spawn-button-not-found-*.png
```

Then update selector in `tests/e2e/agent-communication.spec.ts`

### ❌ "No response received within 30 seconds"

**Fix:** Claude not responding. Check:
1. Is Claude CLI installed? (`claude --version`)
2. Is Claude in PATH?
3. Check screenshot: `07-response-received-*.png`

---

## Screenshots

Every test step saves a screenshot:

```
test-results/screenshots/
├── 01-app-loaded-[timestamp].png
├── 02-spawn-button-not-found-[timestamp].png  (only if button not found)
├── 03-agent-spawned-[timestamp].png
├── 04-input-not-found-[timestamp].png         (only if input not found)
├── 05-message-typed-[timestamp].png
├── 06-message-sent-[timestamp].png
└── 07-response-received-[timestamp].png
```

---

## Test Against Specific Version

```typescript
// Edit tests/e2e/agent-communication.spec.ts

test.beforeAll(async () => {
  tauriApp = await launchTauriApp({
    // Use specific release version
    executablePath: '../../releases/v0.3.1/agentmux-desktop-v0.3.1-portable.exe',
    timeout: 60000,
  });
});
```

---

## Next Steps

1. **Add more tests** - Edit `tests/e2e/agent-communication.spec.ts`
2. **Customize selectors** - Update button/input selectors to match your UI
3. **Add assertions** - Verify specific response content
4. **CI/CD integration** - See `tests/e2e/README.md` for GitHub Actions example

---

## Full Documentation

See: `tests/e2e/README.md`

---

**Time to first test:** ~5 minutes
**Typical test run:** ~1-2 minutes
**Success rate:** High (with robust selectors)
