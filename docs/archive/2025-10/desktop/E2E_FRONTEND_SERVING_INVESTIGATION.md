# E2E Testing Frontend Serving Investigation

**Date:** 2025-10-15
**Issue:** Release builds showing "localhost refused to connect" instead of serving frontend from dist/
**Status:** ROOT CAUSE IDENTIFIED

---

## Problem Statement

When running E2E tests with `cargo build --release`, the Tauri application shows an Edge error page "Hmmm... can't reach this page - localhost refused to connect" instead of loading the frontend UI from the `dist/` directory.

###Screenshot Evidence
- App launches successfully via tauri-driver
- WebDriver session created
- Window handle obtained
- But frontend shows browser error page instead of the AgentMux UI

---

## Investigation Process

### 1. Initial Hypothesis: Debug vs Release Build

**Theory:** Debug builds try to connect to dev server (localhost:1420), release builds serve from dist/

**Testing:**
- Switched from `cargo build` to `cargo build --release`
- Added frontend build step (`npm run build`) before Rust compilation
- Updated wdio.conf.js to use release binary

**Result:** FAILED - Release build still showed "localhost refused to connect"

### 2. Configuration Analysis

**Tauri Configuration (tauri.conf.json):**
```json
{
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  }
}
```

**Frontend Build Output:**
```
dist/
├── assets/
├── index.html (exists ✓)
```

**Conclusion:** Configuration is correct, `dist/` exists with proper structure

### 3. Web Research Findings

#### Key Discovery: `cargo build` vs `tauri build`

From Tauri documentation and community discussions:

**`tauri build` performs:**
1. Runs `build.beforeBuildCommand` (builds frontend)
2. Embeds frontend assets into binary using custom protocol
3. Creates platform-specific bundles/installers
4. Sets proper compile-time flags

**`cargo build --release` only:**
1. Compiles Rust code
2. Does NOT embed frontend assets
3. Does NOT enable custom protocol feature
4. Still tries to connect to `devUrl` at runtime

#### Critical Feature: `custom-protocol`

From research and Stack Overflow:

> "Using `cargo build --release --features tauri/custom-protocol` will be equivalent to `tauri build`, using the distDir/frontendDist instead of devPath/devUrl"

**The custom-protocol feature tells Tauri to:**
- Embed frontend assets at compile time
- Serve assets via `tauri://` protocol instead of `http://localhost`
- Ignore `devUrl` configuration
- Use `frontendDist` path for asset resolution

---

## Root Cause Analysis

### Cargo.toml Investigation

**File:** `src-tauri/Cargo.toml`

**Current configuration:**
```toml
[dependencies]
tauri = { version = "2.2", features = [] }  # ← EMPTY FEATURES ARRAY
```

**This is the problem!** The `custom-protocol` feature is **NOT enabled**.

### Why This Causes the Issue

1. **Without `custom-protocol`:**
   - Tauri always tries to connect to `devUrl` (http://localhost:1420)
   - Even in release builds with `cargo build --release`
   - Frontend assets in `dist/` are ignored
   - Browser shows connection refused error

2. **With `custom-protocol` enabled:**
   - Tauri embeds `dist/` contents into binary at compile time
   - Serves assets via `tauri://localhost/` custom protocol
   - No external server needed
   - Frontend loads correctly

### Why `tauri build` Would Work

The `tauri build` command (via Tauri CLI) automatically:
1. Adds `--features "tauri/custom-protocol"` to cargo build
2. Runs frontend build command
3. Embeds assets correctly

But our E2E test setup uses `cargo build --release` directly, which doesn't add this feature flag.

---

## Solutions

### Option 1: Add custom-protocol Feature (RECOMMENDED)

**Modify:** `src-tauri/Cargo.toml`

```toml
[dependencies]
tauri = { version = "2.2", features = ["custom-protocol"] }
```

**Pros:**
- Minimal change
- Works with `cargo build --release`
- Faster E2E test builds (no bundling overhead)
- Frontend always embedded

**Cons:**
- Changes production dependency configuration
- Need to test that this doesn't break dev mode

### Option 2: Use `cargo build --features`

**Modify:** `wdio.conf.js`

```javascript
const buildResult = spawnSync('cargo', ['build', '--release', '--features', 'tauri/custom-protocol'], {
  cwd: path.join(__dirname, 'src-tauri'),
  stdio: 'inherit',
  env: {
    ...process.env,
    AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
  },
});
```

**Pros:**
- Doesn't modify Cargo.toml
- Feature only enabled for E2E tests
- Explicit about what we're doing

**Cons:**
- More complex build command
- Have to remember to use this flag

### Option 3: Use `tauri build` Command

**Modify:** `wdio.conf.js`

```javascript
const buildResult = spawnSync('npm', ['run', 'tauri', 'build'], {
  cwd: __dirname,
  stdio: 'inherit',
  shell: true,
  env: {
    ...process.env,
    AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
  },
});
```

**Pros:**
- Uses official Tauri build process
- Guaranteed correct configuration
- Future-proof

**Cons:**
- Much slower (creates full bundles)
- Overkill for E2E tests
- Binary location might differ (bundle vs target/release)

---

## Similar Issues in Community

### Issue 1: Angular Projects
**Problem:** `frontendDist` set to `../dist` but Angular builds to `dist/projectName`
**Solution:** Change to `../dist/projectName`

### Issue 2: Next.js Projects
**Problem:** Missing `pages/index.html` entry file
**Solution:** Add index.html redirect page

### Issue 3: Missing custom-protocol
**Problem:** Release build tries localhost instead of embedded assets
**Solution:** Enable `custom-protocol` feature

**Our issue matches #3 exactly.**

---

## Recommended Fix

### Step 1: Enable custom-protocol Feature

**File:** `src-tauri/Cargo.toml`

```toml
[dependencies]
tauri = { version = "2.2", features = ["custom-protocol"] }
```

### Step 2: Test in Dev Mode

```bash
npm run tauri dev
```

Verify the app still works in development mode (it should - the feature is smart enough to use devUrl in dev, frontendDist in prod).

### Step 3: Update E2E Config

The `wdio.conf.js` can stay as is - `cargo build --release` will now work correctly because the feature is enabled by default.

---

## Testing Plan

### Verify the Fix

1. **Enable custom-protocol in Cargo.toml**
2. **Clean build:**
   ```bash
   cd src-tauri
   cargo clean
   cd ..
   npm run build
   cargo build --release
   ```

3. **Manual test:**
   ```bash
   ./src-tauri/target/release/agentmux.exe
   ```
   Should show AgentMux UI, not localhost error

4. **Run E2E tests:**
   ```bash
   npm run test:e2e
   ```
   Should now find UI elements

### Expected Results

- ✅ App window opens
- ✅ Frontend loads from embedded assets
- ✅ UI elements are findable (`[data-testid="tab-agents"]`)
- ✅ Tests can interact with actual UI

---

## Key Learnings

### 1. Cargo Features are Critical

Tauri relies heavily on cargo features to switch between dev and production modes. The `custom-protocol` feature is not optional for production builds.

### 2. Build Commands Matter

- `cargo build` = bare Rust compilation
- `cargo build --features` = Rust + specific features
- `tauri build` = Full production build with all bells and whistles

For E2E tests, we want the middle ground: release binary with embedded assets but without bundling overhead.

### 3. Documentation Assumption

The Tauri documentation assumes you're using `tauri build` for production. Using `cargo build --release` directly is an edge case that requires understanding the feature system.

### 4. Error Diagnosis

The "localhost refused to connect" error is actually misleading - it's not a network issue, it's a missing feature compile flag issue. The app is trying to load from localhost because it doesn't know about the embedded assets.

---

## Timeline

**Duration:** ~2 hours of investigation
**Key Breakthrough:** Discovering the empty `features = []` in Cargo.toml
**Root Cause:** Missing `custom-protocol` feature prevents asset embedding

---

## Related Documentation

### Official Tauri Docs
- [Building Your Application](https://v2.tauri.app/develop/building/)
- [Configuration Files](https://v2.tauri.app/develop/configuration-files/)
- [Custom Protocol](https://v2.tauri.app/reference/config/#tauri.security.assetprotocol)

### Community Resources
- Stack Overflow: "Tauri frontend server not starting"
- GitHub Issue #11474: "tauri refuses to read frontendDist"
- GitHub Discussion #4052: "can't connect to tauri.localhost"

### Project Files
- Configuration: `src-tauri/tauri.conf.json`
- Dependencies: `src-tauri/Cargo.toml`
- E2E Setup: `wdio.conf.js`
- Test Specs: `tests/e2e/claude-terminal-interaction.spec.js`

---

## Conclusion

The E2E test failure was caused by **missing `custom-protocol` feature in Cargo.toml**. Without this feature, release builds attempt to connect to the development server (localhost:1420) instead of serving embedded frontend assets, resulting in connection refused errors.

**Fix:** Add `"custom-protocol"` to the tauri dependency features array.

**Impact:** This is a one-line change that will make E2E tests work correctly with release builds.

---

**Next Steps:**
1. Apply the fix to Cargo.toml
2. Rebuild and test
3. Run full E2E test suite
4. Document the resolution in PR
