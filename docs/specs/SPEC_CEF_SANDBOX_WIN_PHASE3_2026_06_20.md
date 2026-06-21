# SPEC: CEF Windows Renderer Sandbox — Phase 3
**Status:** Approved for implementation
**Issue:** #1374
**Date:** 2026-06-20
**Depends on:** SPEC_CEF_SANDBOX_2026_06_20.md (Phases 1+2, merged PR #1622)

---

## 1. Problem

Phase 1+2 removed `no_sandbox: 1` on macOS (Seatbelt) and Linux (namespace
isolation). Windows is still blocked:

```rust
// agentmux-cef/src/main.rs — Phase 1+2 guard, still present
let no_sandbox: i32 = if cfg!(any(not(feature = "sandbox"), target_os = "windows"))
    || force_no_sandbox
{ 1 } else { 0 };
```

Three root causes:

1. **No `[lib]` target.** CEF's Windows sandbox uses the **DLL wrapper pattern**:
   `bootstrap.exe` (CEF-provided, renamed to `agentmux-cef.exe`) loads the
   application as a DLL, creates sandbox info, then calls the DLL's exported
   `RunWinMain`. Without a `[lib]` cdylib target, there is no DLL to load.

2. **`RunWinMain` export missing.** `bootstrap.exe` calls
   `GetProcAddress(hDll, "RunWinMain")` and aborts with
   `"Failed to find RunWinMain in agentmux-cef.dll"` if the symbol is absent.
   Confirmed from `include/cef_sandbox_win.h` in the CEF distribution and
   from string inspection of `bootstrap.exe`.

3. **`windows_sandbox_info` is null.** Both `CefExecuteProcess` and
   `CefInitialize` are called with `std::ptr::null_mut()`. When the DLL
   receives a valid sandbox info pointer from `bootstrap.exe`, it must pass
   that pointer through.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  dist/cef/agentmux-cef.exe  ← bootstrap.exe (CEF-provided)     │
│  dist/cef/agentmux-cef.dll  ← agentmux_cef.dll (our cdylib)   │
└─────────────────────────────────────────────────────────────────┘
   bootstrap.exe launches, calls:
     1. cef_sandbox_info_create()         → sandbox_info*
     2. LoadLibraryW("agentmux-cef.dll")  → hDll
     3. GetProcAddress(hDll, "RunWinMain")→ fn ptr
     4. RunWinMain(hInst, cmdLine, nShow, sandbox_info, ver_info) → exit code
```

Our `RunWinMain` in `lib.rs` delegates to `run(sandbox_info)` — the same
function that `fn main()` calls with `null_mut()` on non-Windows or sandbox-off builds.

### DLL naming

Rust produces `agentmux_cef.dll` (underscore) from `[lib] name = "agentmux_cef"`.
The `bootstrap.exe` renamed to `agentmux-cef.exe` looks for `agentmux-cef.dll`
(hyphen). Resolve with a Taskfile rename step:

```
target/release/agentmux_cef.dll  →  dist/cef/agentmux-cef.dll
target/release/bootstrap.exe      →  dist/cef/agentmux-cef.exe
```

---

## 3. Entry-point signature

From `include/cef_sandbox_win.h` in the CEF 148 distribution:

```c
// CEF_BOOTSTRAP_EXPORT = __declspec(dllexport) on the DLL side
CEF_BOOTSTRAP_EXPORT int RunWinMain(
    HINSTANCE hInstance,
    LPTSTR    lpCmdLine,
    int       nCmdShow,
    void*     sandbox_info,
    cef_version_info_t* version_info);
```

Rust equivalent (in `lib.rs`):

```rust
#[cfg(all(target_os = "windows", feature = "sandbox"))]
#[no_mangle]
pub unsafe extern "system" fn RunWinMain(
    _h_instance:    windows_sys::Win32::Foundation::HINSTANCE,
    _lp_cmd_line:   *mut u16,
    _n_cmd_show:    i32,
    sandbox_info:   *mut std::ffi::c_void,
    _version_info:  *mut std::ffi::c_void,
) -> i32 {
    run(sandbox_info)
}
```

---

## 4. Implementation plan

### 4.1 `agentmux-cef/Cargo.toml` — add `[lib]` cdylib target

```toml
[lib]
name = "agentmux_cef"
path = "src/lib.rs"
crate-type = ["cdylib"]
```

Keep the existing `[[bin]]` (used for sandbox-off builds via
`--no-default-features` or non-Windows).

### 4.2 `agentmux-cef/src/lib.rs` — new file

Root of the library crate. Contains:
- All `mod` declarations currently in `main.rs`
- All helper functions (`suppress_os_crash_dialogs`, `resolve_browser_subprocess_path`,
  `launcher_is_genuine_parent`, `library_loader` mod usage)
- `pub fn run(windows_sandbox_info: *mut std::ffi::c_void) -> i32` —
  the full body of the current `fn main()`, with:
  - `execute_process(args, None, windows_sandbox_info)` — not `null_mut()`
  - `initialize(args, settings, app, windows_sandbox_info)` — not `null_mut()`
  - `no_sandbox` guard: remove `target_os = "windows"` so Windows+sandbox gets `0`
- `pub unsafe extern "system" fn RunWinMain(...)` (Windows+sandbox only)

The `windows_subsystem = "windows"` cfg_attr stays in `main.rs` (binary attribute,
not valid in a library crate).

### 4.3 `agentmux-cef/src/main.rs` — thin binary wrapper

After the refactor, `main.rs` becomes:

```rust
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hide console window in release mode on Windows (sandbox-off path only).
// The DLL path (Phase 3) uses bootstrap.exe which is already /SUBSYSTEM:WINDOWS.
#![cfg_attr(
    all(not(debug_assertions), not(feature = "sandbox"), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    std::process::exit(agentmux_cef::run(std::ptr::null_mut()));
}
```

### 4.4 `agentmux-cef/src/main.rs` — `no_sandbox` guard update

In `lib.rs`'s `run()`, update the no_sandbox computation:

```rust
// Before (Phase 1+2):
let no_sandbox: i32 = if cfg!(any(not(feature = "sandbox"), target_os = "windows"))
    || force_no_sandbox
{ 1 } else { 0 };

// After (Phase 3):
let no_sandbox: i32 = if cfg!(not(feature = "sandbox")) || force_no_sandbox {
    1
} else {
    0
};
```

Windows with `sandbox` feature now gets `no_sandbox = 0`.

### 4.5 `agentmux-cef/src/main.rs` — pass `sandbox_info` through to CEF

In `run()`:

```rust
// CefExecuteProcess
let ret = execute_process(
    Some(args.as_main_args()),
    None,
    windows_sandbox_info,  // was: std::ptr::null_mut()
);

// CefInitialize
let init_result = initialize(
    Some(args.as_main_args()),
    Some(&settings),
    Some(&mut cef_app),
    windows_sandbox_info,  // was: std::ptr::null_mut()
);
```

### 4.6 `Taskfile.yml` — Windows staging update

In `build:host:windows` (and the bundle/dev equivalents), update the staging
step to detect sandbox mode:

```yaml
# With sandbox (default):
#   bootstrap.exe → dist/cef/agentmux-cef.exe
#   agentmux_cef.dll → dist/cef/agentmux-cef.dll
#   agentmux_cef.pdb → dist/cef/agentmux-cef.pdb (debug info)
# Without sandbox (--no-default-features):
#   agentmux-cef.exe → dist/cef/agentmux-cef.exe (existing path)
```

`bootstrap.exe` is emitted to `target/release/bootstrap.exe` by the
`cef-dll-sys` build script (which already runs `copy_cef_runtime_files`).

Concrete Taskfile change (PowerShell block):

```powershell
if (Test-Path "target/release/agentmux_cef.dll") {
  # Sandbox path: rename DLL to match bootstrap's expected name
  Copy-Item target/release/bootstrap.exe       dist/cef/agentmux-cef.exe -Force
  Copy-Item target/release/agentmux_cef.dll    dist/cef/agentmux-cef.dll -Force
  Copy-Item target/release/agentmux_cef.pdb    dist/cef/agentmux-cef.pdb -Force -EA SilentlyContinue
} else {
  # No-sandbox path: copy the binary directly
  Copy-Item target/release/agentmux-cef.exe    dist/cef/agentmux-cef.exe -Force
}
```

### 4.7 `no_sandbox` guard — `AGENTMUX_UNSAFE_NOSANDBOX` escape hatch

The existing escape hatch (`force_no_sandbox`) already covers Windows —
no additional change needed. When `AGENTMUX_UNSAFE_NOSANDBOX=1` is set,
`no_sandbox = 1` and the sandbox is skipped even if bootstrap.exe provides
non-null sandbox_info. CEF honours `no_sandbox = 1` regardless of what
`sandbox_info` contains.

---

## 5. Files changed

| File | Change |
|------|--------|
| `agentmux-cef/Cargo.toml` | Add `[lib]` cdylib target |
| `agentmux-cef/src/lib.rs` | **New** — all logic from main.rs; `run(sandbox_info)`; `RunWinMain` export |
| `agentmux-cef/src/main.rs` | Shrink to 3-line wrapper calling `agentmux_cef::run(null_mut())` |
| `Taskfile.yml` | Windows staging: bootstrap.exe + DLL rename |
| `.changesets/` | New patch entry |

---

## 6. Testing

| Test | Expected |
|------|----------|
| `cargo check -p agentmux-cef` | compiles both bin and lib targets |
| `cargo check -p agentmux-cef --no-default-features` | bin-only path compiles |
| Windows `task dev`: app starts, browser pane renders | No sandbox regression |
| Windows `task dev`: `chrome://sandbox` | Renderer Sandbox = active |
| Windows agent panes, terminal, tear-off | Unaffected |
| `AGENTMUX_UNSAFE_NOSANDBOX=1` on Windows | Warns in log, sandbox off |
| macOS/Linux `cargo check` with Phase 3 Cargo.toml | Still compiles, sandbox unchanged |

---

## 7. Non-goals

- Changing `bootstrap.exe` or patching the CEF distribution.
- Supporting `RunConsoleMain` — we're `/SUBSYSTEM:WINDOWS` only.
- Flipping `default = ["sandbox"]` for Windows specifically — it's already the
  default and the runtime behaviour changes with Phase 3.
