# Spec: Rename App Identifier to `ai.agentmux`

## Problem

The current Tauri app identifier is `com.agentmuxhq.agentmux` (reverse-domain for a domain that doesn't exist). It should be `ai.agentmux` to match the actual product domain.

The identifier is **versioned** via `.bump.json` template:
```
com.agentmuxhq.agentmux.v0-31-59
```

This versioned pattern is **intentional** — it allows multiple backend versions to run simultaneously, each with its own isolated AppData folder and instance directory. This must be preserved.

Current issues:

1. **Wrong domain** — `com.agentmuxhq` doesn't exist; should be `ai.agentmux`

2. **wsh-rs can't find the backend** — `wsh-rs/src/rpc/mod.rs:42` hardcodes `com.agentmuxhq.agentmux` (no version suffix), but Tauri's `app_config_dir()` resolves to the versioned path. These never match.

3. **Settings button broken** — `settings.json` doesn't exist in a fresh versioned folder, and the opener plugin may lack scope permissions for the path.

## Solution

- Rename identifier from `com.agentmuxhq.agentmux` to `ai.agentmux.app` (keep versioned pattern)
- Fix wsh-rs to use compile-time version (`env!("CARGO_PKG_VERSION")`) to build the correct versioned path
- Fix settings button to create `settings.json` with defaults if it doesn't exist before opening

### Directory Structure (unchanged pattern, new domain)

```
%APPDATA%/ai.agentmux.app.v0-31-60/         <- Tauri app_config_dir (per-version)
  settings.json                              <- created on first launch if missing
  provider-config.json
  instances/
    v0.31.60/
      wave-endpoints.json                    <- backend endpoints for this version
```

Multiple versions can coexist:
```
%APPDATA%/ai.agentmux.app.v0-31-59/         <- older version (may still be running)
%APPDATA%/ai.agentmux.app.v0-31-60/         <- current version
```

## Changes

### 1. `src-tauri/tauri.conf.json`

**Current:**
```json
"identifier": "com.agentmuxhq.agentmux.v0-31-59"
```

**New:**
```json
"identifier": "ai.agentmux.app.v0-31-60"
```

Tauri uses this to compute `app_config_dir()` on all platforms:
- Windows: `%APPDATA%/ai.agentmux.app.v0-31-60/`
- macOS: `~/Library/Application Support/ai.agentmux.app.v0-31-60/`
- Linux: `~/.config/ai.agentmux.app.v0-31-60/`

### 2. `.bump.json`

**Current:**
```json
{
  "file": "src-tauri/tauri.conf.json",
  "type": "json",
  "path": "identifier",
  "transform": "template",
  "template": "com.agentmuxhq.agentmux.v{{version | replace('.', '-')}}"
}
```

**New:**
```json
{
  "file": "src-tauri/tauri.conf.json",
  "type": "json",
  "path": "identifier",
  "transform": "template",
  "template": "ai.agentmux.app.v{{version | replace('.', '-')}}"
}
```

### 3. `package.json`

**Current:**
```json
"appId": "com.agentmuxhq.agentmux"
```

**New:**
```json
"appId": "ai.agentmux.app"
```

### 4. `wsh-rs/src/rpc/mod.rs` — Fix versioned path discovery

**Current (broken):**
```rust
// Hardcoded without version — never matches Tauri's versioned path
.join("com.agentmuxhq.agentmux")
.join("instances")
.join("default")
```

**New:**
```rust
// Use compile-time version to match Tauri's versioned identifier
let version = env!("CARGO_PKG_VERSION").replace('.', "-");
let app_dir = format!("ai.agentmux.app.v{}", version);

// On Windows: %APPDATA%/ai.agentmux.app.v0-31-60/instances/v0.31.60/wave-endpoints.json
// On macOS:   ~/Library/Application Support/ai.agentmux.app.v0-31-60/instances/v0.31.60/...
// On Linux:   ~/.config/ai.agentmux.app.v0-31-60/instances/v0.31.60/...

let version_instance = format!("v{}", env!("CARGO_PKG_VERSION"));
config_dir
    .join(&app_dir)
    .join("instances")
    .join(&version_instance)
    .join("wave-endpoints.json")
```

This fixes the latent bug where wsh could never find the backend's endpoints file.

### 5. Settings button — Create file if missing

**File:** `frontend/app/window/action-widgets.tsx`

**Current:**
```typescript
if (widget.blockdef?.meta?.view === "settings") {
    const path = `${getApi().getConfigDir()}/settings.json`;
    getApi().openNativePath(path);
    return;
}
```

**New:**
```typescript
if (widget.blockdef?.meta?.view === "settings") {
    const path = `${getApi().getConfigDir()}/settings.json`;
    // Ensure settings.json exists before opening
    ensureSettingsFile(path).then(() => {
        getApi().openNativePath(path);
    }).catch(console.error);
    return;
}
```

Add helper using `@tauri-apps/plugin-fs`:
```typescript
import { exists, writeTextFile } from "@tauri-apps/plugin-fs";

async function ensureSettingsFile(path: string): Promise<void> {
    if (!(await exists(path))) {
        const defaults = JSON.stringify({
            // Default settings template
        }, null, 4);
        await writeTextFile(path, defaults);
    }
}
```

### 6. Opener scope — `src-tauri/capabilities/default.json`

Add explicit scope for opening files in the app config directory:

```json
{
  "identifier": "opener:allow-open-path",
  "allow": [{ "path": "$APPCONFIG/**" }]
}
```

### 7. `src-tauri/src/crash.rs`

**Current (line 39):**
```rust
"Please report this at: https://github.com/agentmuxhq/agentmux/issues\n"
```

No change needed — this is the GitHub org URL, not the app identifier.

## Files NOT Changed (historical/docs only)

These files contain `com.agentmuxhq` in historical context and should not be updated:
- `specs/archive/rebrand.md` - historical migration spec
- `specs/archive/tauri-migration-complete.md` - historical spec
- `SECURITY_CLEANUP_REPORT_2026-03-03.md` - historical report

## Verification

1. Build and launch the app
2. Verify `%APPDATA%/ai.agentmux.app.v0-31-XX/` is created (not `com.agentmuxhq.*`)
3. Click settings button — verify `settings.json` is created and opened
4. Verify `wsh-rs` can find and connect to the backend via the versioned path
5. Verify version bumps update the identifier correctly
6. Launch two different versions simultaneously — verify they use separate AppData folders

## Impact

- **Breaking:** Users on old builds will have a new AppData directory. Pre-release, so acceptable.
- **wsh-rs fix:** Versioned path lookup fixes a bug where wsh could never find endpoints.
- **Settings fix:** Creating the file on first click + opener scope fixes the settings button.
- **Multi-version:** Parallel backend instances continue to work as designed.
