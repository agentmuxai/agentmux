# E2E Testing Setup Guide

## ⚠️ Migration from Playwright to tauri-driver

This project has switched from Playwright to tauri-driver for E2E testing.

**Why?** Tauri's WebView2 integration doesn't respect `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` the same way standalone WebView2 apps do, causing Playwright's CDP approach to fail. tauri-driver is the official Tauri testing solution and uses msedgedriver directly.

See `_temp/TAURI_E2E_TESTING_RESEARCH.md` for full analysis.

---

## Prerequisites

### 1. Install tauri-driver (Rust)

```bash
cargo install tauri-driver
```

Verify:
```bash
tauri-driver --version
```

### 2. Install msedgedriver (WebView2 Driver)

Download from: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/

**Match your Edge/WebView2 version:**
```powershell
# Check Edge version
(Get-AppxPackage -Name "Microsoft.Edge").Version
```

Extract `msedgedriver.exe` and add to PATH.

### 3. Install npm dependencies

```bash
npm install
```

### 4. Build Tauri app (release mode)

```bash
npm run tauri:build
```

---

## Running Tests

```bash
npm run test:e2e
```

---

## How It Works

1. `wdio.conf.js` starts tauri-driver server
2. tauri-driver uses msedgedriver to automate WebView2
3. WebdriverIO test client connects and runs tests
4. Tests exercise the Tauri app UI

---

## Troubleshooting

**"tauri-driver: command not found"**
→ Install: `cargo install tauri-driver`

**"msedgedriver.exe not found"**
→ Download and add to PATH

**Tests timeout**
→ Increase `connectionRetryTimeout` in wdio.conf.js

---

## References

- [Tauri Testing Guide](https://tauri.app/v1/guides/testing/webdriver/)
- [WebdriverIO Docs](https://webdriver.io/docs/gettingstarted)
