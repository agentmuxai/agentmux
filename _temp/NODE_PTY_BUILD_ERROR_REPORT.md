# node-pty Windows Build Error - Comprehensive Report

**Date:** 2025-10-15
**Error:** GetCommitHash.bat is not recognized during node-gyp rebuild
**Project:** AgentMux Desktop
**Version:** node-pty 1.0.0

---

## Error Summary

```
'GetCommitHash.bat' is not recognized as an internal or external command,
operable program or batch file.
gyp: Call to 'cmd /c "cd shared && GetCommitHash.bat"' returned exit status 1
while in deps\winpty\src\winpty.gyp. while loading dependencies of binding.gyp
```

## Root Cause

The error occurs during the node-gyp configure phase when building the winpty dependency for node-pty on Windows. The `GetCommitHash.bat` script exists at `deps/winpty/src/shared/GetCommitHash.bat` but cannot be executed by the gyp build system due to one of the following issues:

1. **Current Working Directory Issue**: gyp runs the command from a directory where the relative path "shared\GetCommitHash.bat" doesn't resolve correctly
2. **PATH Environment Issue**: CMD cannot find `.bat` files unless they're in the current directory or specified with full path
3. **Missing Build Dependencies**: Visual Studio 2022 Spectre-mitigated libraries are required but not installed

## Environment Details

- **OS:** Windows 10.0.26200
- **Node.js:** v22.19.0
- **npm:** 10.9.3
- **node-gyp:** 11.2.0 (npm bundled), 11.4.2 (global)
- **Python:** 3.12.10
- **Visual Studio:** 2022 Community (17.14.36518.9)
- **Git:** Installed and working (GetCommitHash.bat runs successfully when executed directly)

## Investigation Results

### 1. File Exists and Executes Correctly

```bash
$ cd node_modules/node-pty/deps/winpty/src/shared
$ ./GetCommitHash.bat
90468a77382722b7f902d46a7f0441a3c8786b32  # ✅ Works!
```

### 2. Winpty.gyp Configuration

The problematic lines in `deps/winpty/src/winpty.gyp`:

```gyp
'variables': {
    'WINPTY_COMMIT_HASH%': '<!(cmd /c "cd shared && GetCommitHash.bat")',
},
'include_dirs': [
    '<!(cmd /c "cd shared && UpdateGenVersion.bat <(WINPTY_COMMIT_HASH)")',
],
```

### 3. Attempted Fixes

#### Fix #1: Add `call` keyword
```gyp
'WINPTY_COMMIT_HASH%': '<!(cmd /c "cd shared && call GetCommitHash.bat")',
```
**Result:** ❌ Same error

#### Fix #2: Use direct path without cd
```gyp
'WINPTY_COMMIT_HASH%': '<!(cmd /c "shared\\GetCommitHash.bat")',
```
**Result:** ❌ Same error

### 4. Research Findings

From web search and GitHub issues:

#### Common Causes:
1. **Missing Spectre-mitigated libraries** (VS 2022)
   - Required component: "MSVC v143 - VS 2022 C++ x64/x86 Spectre-mitigated libs"
   - Install via Visual Studio Installer → Individual components

2. **windows-build-tools deprecated**
   - Old solution: `npm install --global --production windows-build-tools`
   - Status: Now deprecated for Node.js 12+

3. **PATH or working directory issues**
   - gyp executes commands in a specific context where relative paths may not resolve
   - CMD .bat files need explicit execution context

#### Successful Solutions from Community:

1. **Install Spectre-mitigated libraries**
   - Open Visual Studio Installer
   - Modify VS 2022 installation
   - Individual components → Install "MSVC v143 - VS 2022 C++ x64/x86 Spectre-mitigated libs (Latest)"

2. **Use Pre-compiled Binaries**
   - node-pty provides prebuilt binaries for common platforms
   - May avoid compilation entirely

3. **Alternative PTY Libraries**
   - Use Windows ConPTY API directly (Windows 10 1809+)
   - node-pty automatically uses ConPTY on newer Windows versions

## Recommended Solutions

### Solution 1: Install Missing VS Components (RECOMMENDED)

```powershell
# Open Visual Studio Installer
# Modify "Visual Studio Community 2022"
# Go to "Individual components" tab
# Search for and install:
# ✅ MSVC v143 - VS 2022 C++ x64/x86 Spectre-mitigated libs (v14.38-17.8)
# ✅ MSVC v143 - VS 2022 C++ ARM64/ARM64EC Spectre-mitigated libs (Latest)
# ✅ C++ ATL for latest v143 build tools with Spectre Mitigations (x86 & x64)
```

**Rationale:** This is the most common fix reported in GitHub issues for VS 2022.

### Solution 2: Use node-pty Prebuilt Binaries

Check if prebuilt binaries are available for your platform:

```bash
npm install node-pty --force
```

The `--force` flag may trigger downloading prebuilt binaries instead of compiling.

### Solution 3: Modify gyp File to Use Absolute Paths

```gyp
'variables': {
    'WINPTY_COMMIT_HASH%': '<!(python -c "import subprocess; subprocess.call([\"git\", \"rev-parse\", \"HEAD\"])")',
},
```

Replace batch scripts with cross-platform Python commands that gyp can execute reliably.

### Solution 4: Use Alternative Implementation (STRATEGIC)

Since AgentMux targets Windows 10+, we can bypass winpty entirely:

```javascript
// Use Node.js native child_process with Windows ConPTY
const { spawn } = require('child_process');
const child = spawn('powershell.exe', [], {
  stdio: ['pipe', 'pipe', 'pipe'],
  windowsHide: true,
});
```

**Benefits:**
- No native compilation required
- Faster deployment
- Fewer build dependencies
- Windows ConPTY is built into modern Windows

## Next Steps

1. **Immediate:** Install Spectre-mitigated libraries in Visual Studio 2022
2. **Short-term:** Retry node-pty installation
3. **Long-term:** Consider implementing direct ConPTY support or using Rust PTY library (portable-pty)

## Related Files

- Error location: `D:\Code\WebProjects\agentmux\apps\desktop\wrappers\node_modules\node-pty\deps\winpty\src\winpty.gyp`
- Script location: `D:\Code\WebProjects\agentmux\apps\desktop\wrappers\node_modules\node-pty\deps\winpty\src\shared\GetCommitHash.bat`
- Root cause document: `D:\Code\WebProjects\agentmux\_temp\CLAUDE_NO_RESPONSE_ROOT_CAUSE.md`

## References

- Microsoft node-pty: https://github.com/microsoft/node-pty
- Issue #645: Update Windows Installation Prerequisite
- Issue #439: node-pty can't be installed
- ConPTY API: https://docs.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session

---

## Solution Implemented

### Fix Applied: Hardcoded Version Info

The GetCommitHash.bat error was resolved by bypassing the batch scripts entirely:

1. **Modified winpty.gyp:**
   - Changed `'WINPTY_COMMIT_HASH%': '<!(cmd /c "cd shared && GetCommitHash.bat")'` to `'WINPTY_COMMIT_HASH%': 'none'`
   - Changed `'<!(cmd /c "cd shared && UpdateGenVersion.bat <(WINPTY_COMMIT_HASH)")'` to `'gen'`

2. **Created GenVersion.h manually:**
   ```c
   // AUTO-GENERATED (manually created to bypass build script issues)
   const char GenVersion_Version[] = "0.4.4-dev";
   const char GenVersion_Commit[] = "none";
   ```

3. **Result:** ✅ gyp configure phase passed successfully!

### New Issue Discovered

After bypassing the batch script issue, compilation now fails with ConPTY API errors:

```
error C2065: 'PFNCREATEPSEUDOCONSOLE': undeclared identifier
error C2065: 'PFNRESIZEPSEUDOCONSOLE': undeclared identifier
error C2065: 'PFNCLEARPSEUDOCONSOLE': undeclared identifier
error C2065: 'PFNCLOSEPSEUDOCONSOLE': undeclared identifier
```

This indicates Windows SDK headers are missing ConPTY API definitions. Likely need Windows SDK 10.0.17763.0 or newer.

**Status:** node-pty build in progress, resolving ConPTY API compilation errors
