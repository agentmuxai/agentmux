# E2E Testing - Next Steps

**Date:** 2025-10-15
**Status:** tauri-driver installed, msedgedriver needed

---

## ✅ Completed

1. **tauri-driver installed**
   - Version: 2.0.4
   - Location: `C:\Users\asafe\.cargo\bin\tauri-driver.exe`
   - Command works: `tauri-driver -h`

2. **WebdriverIO dependencies installed**
   - All npm packages installed successfully
   - wdio.conf.js configuration created

3. **Test files migrated**
   - All test helpers rewritten for WebdriverIO
   - Test spec ready: `claude-terminal-interaction.spec.js`

---

## ⚠️ Blocking Issue: msedgedriver Missing

Tests cannot run without msedgedriver.

### Your Edge Version
```
140.0.3485.54
```

### Download msedgedriver

**Option 1: Direct Download (Recommended)**

1. Go to: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/
2. Select version: **140.0.3485.54** (MUST match your Edge version)
3. Download for Windows x64
4. Extract `msedgedriver.exe`
5. Add to PATH or place in project root

**Option 2: Automated Download (if available)**

```powershell
# PowerShell script to download and extract msedgedriver
$version = "140.0.3485.54"
$url = "https://msedgedriver.azureedge.net/$version/edgedriver_win64.zip"
$output = "$env:TEMP\edgedriver.zip"

Invoke-WebRequest -Uri $url -OutFile $output
Expand-Archive -Path $output -DestinationPath "." -Force
Remove-Item $output
```

### Verify Installation

After downloading, verify:

```bash
# Check if msedgedriver is accessible
msedgedriver.exe --version

# Should output: MSEdgeDriver 140.0.3485.54 (or similar)
```

---

## Running Tests (After msedgedriver Setup)

### 1. Ensure release build exists

```bash
cd D:\Code\WebProjects\agentmux\apps\desktop
npm run tauri:build
```

Executable should be at:
```
src-tauri/target/release/agentmux.exe
```

### 2. Run E2E tests

```bash
npm run test:e2e
```

### 3. Expected Test Output

```
[wdio] Starting tauri-driver server...
[wdio] ✓ tauri-driver server started

[Tauri E2E] Waiting for app to be ready...
[Tauri E2E] ✓ App ready
[Claude E2E] Spawning Claude agent...
[Claude E2E] ✓ Claude agent spawned

Spec Files:      1 passed, 1 total
Tests:           4 passed, 4 total
Duration:        ~2 minutes
```

---

## Troubleshooting

### "msedgedriver.exe not found"
→ Download and add to PATH (see above)

### "Failed to start tauri-driver"
→ Check logs for specific error
→ Ensure tauri-driver is in PATH: `which tauri-driver`

### "Cannot find executable"
→ Build release: `npm run tauri:build`
→ Check path in wdio.conf.js

### "Version mismatch"
→ msedgedriver version MUST match Edge version (140.0.3485.54)
→ Re-download correct version

### Tests timeout
→ Increase `connectionRetryTimeout` in wdio.conf.js
→ Check if app launches manually

---

## Alternative: Use Different Edge Version

If downloading msedgedriver 140.0.3485.54 fails, you can:

1. Check available versions: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/
2. Download the closest available version
3. Tests may still work with minor version mismatch (e.g., 140.0.3485.X)

---

## Summary

**What's working:**
- ✅ tauri-driver installed
- ✅ WebdriverIO configured
- ✅ Test files ready
- ✅ Edge detected (140.0.3485.54)

**What's blocking:**
- ❌ msedgedriver not installed

**Action required:**
Download msedgedriver 140.0.3485.54 and add to PATH, then run `npm run test:e2e`

---

## Resources

- msedgedriver downloads: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/
- Tauri WebDriver guide: https://tauri.app/v1/guides/testing/webdriver/
- WebdriverIO docs: https://webdriver.io/docs/gettingstarted
