# Spec: Sidecar Isolation per AgentMux Version

**Status:** Draft
**Author:** AgentA
**Date:** 2026-03-21

---

## Component Overview

AgentMux has four distinct runtime components:

```
┌─────────────────────────────────────────────────────────┐
│  agentmux.exe  (Tauri host — one per window)            │
│  • Manages WebView2 windows                             │
│  • Handles Tauri IPC (invoke/listen)                    │
│  • Spawns and supervises sidecars                       │
│  • Identifier: ai.agentmux.app.v0-32-63 (versioned)    │
├─────────────────────────────────────────────────────────┤
│  WebView2 (frontend)                                    │
│  • SolidJS/TypeScript UI                                │
│  • Runs inside the Tauri window                         │
│  • Isolated WebView2 UDF per version                    │
├─────────────────────────────────────────────────────────┤
│  agentmuxsrv-rs.exe  (Sidecar 1 — backend server)      │
│  • WebSocket RPC server                                 │
│  • Block controllers (terminal, subprocess, agent)      │
│  • State store (wave objects)                           │
│  • Spawned by Tauri on first window open                │
│  • One instance per AgentMux session                    │
├─────────────────────────────────────────────────────────┤
│  wsh.exe  (Sidecar 2 — shell integration)               │
│  • Injected into terminal panes                         │
│  • Reports pane title/color/env back to backend         │
│  • Called per-terminal, not a long-running daemon       │
│  • Versioned in dist: wsh-0.32.63-windows.x64.exe       │
└─────────────────────────────────────────────────────────┘
```

---

## What Is Already Isolated (Working Today)

| Component | Isolation Mechanism | Status |
|-----------|--------------------|-|
| `agentmux.exe` | Unique Tauri identifier per version (`ai.agentmux.app.v0-32-63`) → separate WebView2 UDF, separate app data dir | ✅ Works |
| WebView2 frontend | Separate UDF per identifier, separate cache/storage | ✅ Works |
| Auth dirs | `{app_data_dir}/auth/{provider}/` — version-isolated via identifier | ✅ New (this PR) |
| `wsh` in `dist/bin/` | Versioned filename: `wsh-0.32.63-windows.x64.exe` | ✅ Works |

---

## What Is NOT Isolated (The Problem)

Tauri's `externalBin` sidecar mechanism requires binaries in `src-tauri/binaries/` to follow a strict naming convention:

```
src-tauri/binaries/
  agentmuxsrv-rs-x86_64-pc-windows-msvc.exe   ← FIXED name, ALL versions
  wsh-x86_64-pc-windows-msvc.exe               ← FIXED name, ALL versions
```

Tauri's `tauri.conf.json`:
```json
"externalBin": [
  "binaries/agentmuxsrv-rs",
  "binaries/wsh"
]
```

The name `agentmuxsrv-rs-{target-triple}` is resolved at **compile time** by `tauri-build`. It cannot be dynamic. This means:

1. Every version builds to the SAME filename in the source tree
2. `tauri-build` reads/hashes the sidecar binary during compilation
3. On Windows, if the sidecar is running (loaded by the previous version), it has an **exclusive read lock**
4. `tauri-build` gets `PermissionDenied` → build fails with exit code 101

### Why This Matters for Parallel Versions

- v0.32.60 is running → `agentmuxsrv-rs-x86_64-pc-windows-msvc.exe` is locked
- Building v0.32.63 → `tauri-build` tries to hash the same file → ❌ `PermissionDenied`

This is a **Windows-only** problem. On Linux/macOS, executables can be read/replaced while running.

---

## Root Cause: Tauri Build Script Behavior

`tauri-build` (v2.5.6, `lib.rs:80`) iterates `externalBin` entries and calls `std::fs::metadata()` or reads the file to emit `cargo:rerun-if-changed`. On Windows this requires a read handle, which is blocked if the file is open for execution.

---

## Proposed Solution: Version-Stamped Sidecar in App Data Dir

### Concept

Instead of running the sidecar directly from `src-tauri/binaries/`, copy it to the version-isolated app data dir at startup and run it from there. The file in `src-tauri/binaries/` becomes a **staging area** that gets copied on first launch.

```
src-tauri/binaries/
  agentmuxsrv-rs-x86_64-pc-windows-msvc.exe   ← staging (build artifact)

~/.agentmux-v0-32-63/                           ← Tauri app data (versioned)
  sidecar/
    agentmuxsrv-rs.exe                          ← running copy (version-isolated)
```

The copy happens once at startup (if missing or if staging is newer). The running instance owns its copy. Building a new version only touches the staging file — no conflict.

### Build Step Change

`task build:backend` already copies to `dist/bin/agentmuxsrv-rs.x64.exe` and `src-tauri/binaries/agentmuxsrv-rs-x86_64-pc-windows-msvc.exe`. No change needed there.

### Tauri Setup Change

In `src-tauri/src/lib.rs` `setup()`, before spawning the sidecar:

```rust
// Copy sidecar to versioned app data dir if not already present
fn ensure_versioned_sidecar(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let data_dir = app.path().app_data_dir()
        .map_err(|e| format!("data dir: {e}"))?;
    let sidecar_dir = data_dir.join("sidecar");
    std::fs::create_dir_all(&sidecar_dir).ok();

    let dest = sidecar_dir.join("agentmuxsrv-rs.exe");  // or platform-appropriate name

    if !dest.exists() {
        // Copy from the bundle resource
        let src = app.path().resource_dir()
            .map_err(|e| format!("resource dir: {e}"))?
            .join("binaries")
            .join("agentmuxsrv-rs-x86_64-pc-windows-msvc.exe");
        std::fs::copy(&src, &dest)
            .map_err(|e| format!("copy sidecar: {e}"))?;
    }

    Ok(dest)
}
```

Then spawn `agentmuxsrv-rs` from `dest` instead of using Tauri's built-in sidecar mechanism.

---

## Alternative: Tauri Build Script Patch

Modify `build.rs` in `src-tauri/` to skip hashing sidecars that are already running (Windows only). This is more fragile and fights against tauri-build internals.

**Not recommended.**

---

## Alternative: Side-by-Side Sidecar Names via Build Script

Use a Cargo build script (`build.rs`) to rename the sidecar to include the version:

```
src-tauri/binaries/
  agentmuxsrv-rs-0.32.63-x86_64-pc-windows-msvc.exe
```

And in `tauri.conf.json` use a template variable for the version. Tauri 2.x does not natively support templated `externalBin` names, so this would require patching `tauri-build` or using a wrapper script.

**Not recommended** — too much friction.

---

## Recommended Implementation Plan

### Phase 1 (Immediate Unblock)
Kill orphaned dev sidecars before building. Document this in `CLAUDE.md` as a known Windows build constraint until Phase 2 is shipped.

### Phase 2 (Proper Fix)
1. In `setup()`, copy `agentmuxsrv-rs` to `{app_data_dir}/sidecar/agentmuxsrv-rs.exe` on first launch
2. Modify sidecar spawn code to use the app data copy
3. Update `tauri.conf.json`: keep `externalBin` for bundle packaging, but spawn from the copy at runtime
4. Add version hash check: if the staging binary is newer than the copy, update the copy on next launch

### Phase 3 (wsh)
Apply the same treatment to `wsh` — currently partially versioned in `dist/bin/` but `src-tauri/binaries/wsh-{triple}` still uses the fixed name.

---

## Impact of Current Issue

- **Build blocking**: Can't build a new version while the same version's sidecar is running on Windows
- **Workaround**: Kill the orphaned sidecar (`agentmuxsrv-rs.exe`, not `.x64.exe`) before building
- **Does NOT affect**: The running 0.32.60 instance — its `agentmuxsrv-rs.x64.exe` in `dist/bin/` is unaffected

---

## Open Questions

1. Should Phase 2 copy on every launch or only when the version changes? (hash comparison is safer)
2. Should the sidecar copy be in `{app_data_dir}/sidecar/` or `{resource_dir}/`? (app data is writable, resource dir may not be on all platforms)
3. Does the same problem affect `agentmux.exe` itself? (No — Tauri doesn't hash its own executable in the build script)
