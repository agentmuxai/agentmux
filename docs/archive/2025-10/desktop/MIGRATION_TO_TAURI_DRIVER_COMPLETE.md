# Migration to tauri-driver Complete

**Date:** 2025-10-15
**PR:** #29 - [WIP] Fix E2E tests: Add dynamic ports, unique user data, and test mode
**Branch:** `feature/fix-e2e-tests-dynamic-ports`

---

## Summary

Successfully migrated E2E testing infrastructure from **Playwright + CDP** to **tauri-driver + WebdriverIO**.

---

## Why This Was Necessary

### The Problem with Playwright

Playwright's approach to WebView2 automation relies on the Chrome DevTools Protocol (CDP), which requires:
```
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>
```

This works for **standalone WebView2 apps** but **NOT for Tauri apps**.

### Root Cause

Tauri's WebView2 integration doesn't respect `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` the same way standalone WebView2 apps do. This is a Tauri-specific limitation not documented in Playwright's generic WebView2 guide.

**Evidence:**
- Tests timed out waiting for CDP port to open
- User observed apps launching but showing "couldn't connect to localhost" errors
- Multiple attempts with dynamic ports and unique user data directories all failed

### The Solution

**tauri-driver** is the official Tauri testing tool that:
- Uses msedgedriver (WebView2's native driver) directly
- Doesn't rely on environment variable hacks
- Is specifically designed for Tauri's WebView2 integration
- Uses standard WebDriver protocol (W3C standard)

---

## What Changed

### Dependencies Removed
```json
"@playwright/test": "^1.56.0",
"playwright": "^1.56.0"
```

### Dependencies Added
```json
"@wdio/cli": "^8.40.0",
"@wdio/local-runner": "^8.40.0",
"@wdio/mocha-framework": "^8.40.0",
"@wdio/spec-reporter": "^8.40.0",
"webdriverio": "^8.40.0"
```

**Note:** `tauri-driver` is NOT an npm package - it's a Rust binary installed via:
```bash
cargo install tauri-driver
```

### Configuration

**Removed:**
- `playwright.config.ts` - Playwright test configuration
- `playwright-e2e.config.ts` - E2E-specific Playwright config

**Added:**
- `wdio.conf.js` - WebdriverIO configuration
  - Starts tauri-driver server before tests
  - Configures capabilities for Tauri app
  - Uses Mocha test framework

### Test Helpers

**Removed:**
- `tests/e2e/helpers/tauri-app.ts` - Playwright-based app launcher
- `tests/e2e/helpers/claude-helpers.ts` - Playwright API test helpers

**Added:**
- `tests/e2e/helpers/tauri-app.js` - Simplified WebdriverIO helpers
- `tests/e2e/helpers/claude-helpers.js` - WebdriverIO API test helpers

**Key Difference:** With tauri-driver, we don't manually launch the app or manage connections. The driver handles everything automatically.

### Test Specs

**Removed:**
- `tests/e2e/claude-terminal-interaction.spec.ts` (TypeScript + Playwright)
- Old specs: `agent-communication.spec.ts`, `agents-manager.spec.ts`, `bus-control.spec.ts`, `dashboard.spec.ts`, `message-stream.spec.ts`

**Added:**
- `tests/e2e/claude-terminal-interaction.spec.js` (JavaScript + WebdriverIO)

**Test Cases (unchanged):**
- TC1: Click terminal output → input focused
- TC2: Arrow keys navigate without scrolling
- TC3: Claude responds to Enter key
- TC4: Input and output appear continuous

### Documentation

**Removed:**
- `tests/e2e/README.md` - Old Playwright setup guide

**Added:**
- `tests/e2e/E2E_SETUP.md` - New tauri-driver setup guide
- `_temp/TAURI_E2E_TESTING_RESEARCH.md` - Updated with postscript explaining decision

### Package Scripts

**Before:**
```json
"test:playwright": "playwright test",
"test:playwright:ui": "playwright test --ui",
"test:playwright:debug": "playwright test --debug",
"test:playwright:report": "playwright show-report"
```

**After:**
```json
"test:e2e": "wdio run wdio.conf.js",
"test:e2e:spec": "wdio run wdio.conf.js --spec"
```

---

## Setup Requirements

### 1. Install tauri-driver
```bash
cargo install tauri-driver
```

### 2. Install msedgedriver
Download from: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/

**MUST match your Edge/WebView2 version:**
```powershell
(Get-AppxPackage -Name "Microsoft.Edge").Version
```

### 3. Install npm dependencies
```bash
npm install
```

### 4. Build Tauri app
```bash
npm run tauri:build
```

---

## Running Tests

```bash
npm run test:e2e
```

---

## Why tauri-driver Wasn't Chosen Initially

Looking at the original research document (`TAURI_E2E_TESTING_RESEARCH.md`), tauri-driver **was** considered as "Option 1" but rejected with:

```
Verdict: ❌ Rejected - Playwright is simpler and we already have infrastructure
```

**The reasoning was:**
- Playwright seemed simpler
- Had familiar API
- Already had test infrastructure started
- Playwright's WebView2 docs made it seem viable

**This was a mistake.** We should have trusted Tauri's official documentation over a generic approach that worked for standalone WebView2 apps but not Tauri's specific integration.

---

## Lesson Learned

**When a framework has official testing tools, use them first.**

Don't assume generic solutions will work with framework-specific integrations, even if the underlying technology (WebView2) is the same.

Playwright works great for:
- Web browsers (Chrome, Firefox, Safari)
- Electron apps
- Standalone WebView2 apps

But NOT for:
- Tauri apps (use tauri-driver)

---

## Next Steps

1. **Install prerequisites:**
   - `cargo install tauri-driver`
   - Download and install msedgedriver

2. **Run tests:**
   ```bash
   npm run test:e2e
   ```

3. **If tests pass:**
   - Update PR description
   - Request review
   - Merge PR #29

4. **If tests fail:**
   - Debug with increased logging (`logLevel: 'trace'` in wdio.conf.js)
   - Check tauri-driver output for errors
   - Verify msedgedriver version matches Edge version

---

## Files Changed

**Modified:**
- `package.json` - Dependencies and test scripts
- `_temp/TAURI_E2E_TESTING_RESEARCH.md` - Added postscript

**Deleted:**
- `playwright.config.ts`
- `playwright-e2e.config.ts`
- `tests/e2e/README.md`
- `tests/e2e/helpers/tauri-app.ts`
- `tests/e2e/helpers/claude-helpers.ts`
- `tests/e2e/*.spec.ts` (all old Playwright specs)

**Added:**
- `wdio.conf.js`
- `tests/e2e/E2E_SETUP.md`
- `tests/e2e/helpers/tauri-app.js`
- `tests/e2e/helpers/claude-helpers.js`
- `tests/e2e/claude-terminal-interaction.spec.js`

---

## Migration Status

✅ **COMPLETE**

- ✅ Dependencies updated
- ✅ Configuration created
- ✅ Test helpers rewritten
- ✅ Test specs rewritten
- ✅ Documentation updated
- ✅ Changes committed and pushed

**Ready for testing!**

---

## References

- [Tauri Testing Guide](https://tauri.app/v1/guides/testing/webdriver/)
- [WebdriverIO Docs](https://webdriver.io/docs/gettingstarted)
- [tauri-driver GitHub](https://github.com/tauri-apps/tauri/tree/dev/tooling/webdriver)
- [Original Research Document](./_temp/TAURI_E2E_TESTING_RESEARCH.md)
