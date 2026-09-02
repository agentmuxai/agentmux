# CEF Portable Layout — Clean Directory Spec

**Status:** Ready to implement
**Branch:** agentx/cef-integration
**Goal:** Users extract ZIP, see only `agentmux.exe` + `README.txt`, double-click and go.

---

## Target Layout

```
agentmux-0.32.110-x64-portable/
  agentmux.exe                    2.5 MB   ← only exe the user sees
  README.txt                               ← quick start instructions
  runtime/                                 ← everything else
    agentmuxsrv-rs.x64.exe        8.6 MB   sidecar
    libcef.dll                   251   MB   CEF core
    chrome_elf.dll                 2.4 MB   crash reporting
    libEGL.dll                     0.5 MB   GPU (ANGLE)
    libGLESv2.dll                  8   MB   GPU (ANGLE)
    d3dcompiler_47.dll             4.6 MB   GPU (D3D)
    icudtl.dat                    11   MB   Unicode data
    v8_context_snapshot.bin        0.7 MB   V8 startup
    chrome_100_percent.pak         0.7 MB   UI resources @1x
    chrome_200_percent.pak         1.2 MB   UI resources @2x
    resources.pak                 18   MB   Chromium resources
    locales/
      en-US.pak                    0.4 MB
    frontend/
      index.html
      assets/
      fontawesome/
      fonts/
```

---

## Current State: 90% Done

The runtime/ subdirectory infrastructure was built in commit `ed9a7bc`, then reverted
to flat layout in `e8635da` (43 minutes later — incomplete testing, not a design flaw).
Most of the code still exists and works.

### What Already Works

| Component | File | Status |
|-----------|------|--------|
| DLL loading from `runtime/` | `main.rs:37-54` | Done — `SetDllDirectoryW` called if `runtime/` exists |
| CEF resources from `runtime/` | `main.rs:170-183` | Done — `resources_dir_path` set to `runtime/` if it exists |
| CEF locales from `runtime/locales/` | `main.rs:170-183` | Done — `locales_dir_path` set if `runtime/` exists |
| Sidecar from `runtime/` | `sidecar.rs:268-275` | Done — checks `runtime/{name}.x64.exe` first |
| Frontend from `runtime/frontend/` | `ipc.rs:68-75` | **Reverted** — currently only checks `exe_dir/frontend/` |

### What Needs to Change

**1. ipc.rs — restore runtime/frontend/ probe (8 lines)**

Current (flat only):
```rust
let frontend_dir = exe_dir.join("frontend");
```

Change to:
```rust
let runtime_dir = exe_dir.join("runtime");
let frontend_dir = if runtime_dir.join("frontend/index.html").exists() {
    runtime_dir.join("frontend")
} else {
    exe_dir.join("frontend")
};
```

This exact code existed in commit `ed9a7bc`. Falls back to flat layout for dev mode.

**2. Taskfile — update cef:package:portable (assembly paths)**

Change all `Copy-Item ... $portableDir` to `Copy-Item ... $runtimeDir` for everything
except `agentmux.exe` and `README.txt`.

**3. README.txt — create in packaging task**

```
AgentMux v{VERSION} — Portable Edition

Quick Start:
  1. Extract this ZIP to any folder
  2. Run agentmux.exe

Requirements:
  - Windows 10/11 x64
  - No installation needed
  - No admin rights required

The runtime/ folder contains application internals.
Do not modify or delete files in it.
```

---

## Why It Will Work This Time

The previous attempt wasn't broken — it was reverted before testing completed.
The runtime/ approach was replaced with "flat layout" as a simplification during
rapid iteration. The underlying code was never removed:

- `main.rs` still calls `SetDllDirectoryW(runtime/)` if the dir exists
- `main.rs` still sets `resources_dir_path` / `locales_dir_path` to `runtime/`
- `sidecar.rs` still checks `runtime/{name}.x64.exe` before flat layout

The only code that was removed was the `ipc.rs` frontend probe — an 8-line change.

---

## Compatibility

| Layout | agentmux.exe | Dev mode (`cef:run`) | Portable |
|--------|-------------|---------------------|----------|
| Flat (current) | Works | Works | Works |
| runtime/ (this spec) | Works | Still works (falls back to flat) | Works |

The fallback chain in every component checks runtime/ first, then flat.
Dev mode (`task cef:dev`) uses flat layout (no `runtime/` dir in `dist/cef/`),
so it keeps working without changes.

---

## Implementation Steps

1. Restore `runtime/frontend/` probe in `ipc.rs` (8 lines)
2. Update `cef:package:portable` in Taskfile to assemble into `runtime/`
3. Add README.txt generation to the packaging task
4. Build, package, test:
   - `task cef:build && task cef:bundle && task build:frontend && task cef:package:portable`
   - Extract ZIP, run `agentmux.exe`
   - Verify: terminal opens, backend connects, panes work
5. Verify dev mode still works: `task cef:dev`
