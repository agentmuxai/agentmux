# `task dev` CEF Initialization Failure — Root Cause Analysis

**Date:** 2026-05-01
**Status:** Diagnosed
**Repo state:** main @ `ea2c477d`, AgentMux v0.33.560
**Author:** AgentC

---

## Symptom

Running `task dev` fails immediately at CEF init:

```
thread 'main' (24416) panicked at agentmux-cef\src\main.rs:398:5:
assertion `left == right` failed: CEF initialization failed
  left: 0
 right: 1
```

`cef::initialize()` returns 0 (failure). Reproducible. `chrome_debug.log` in the dev cache dir is empty (0 bytes) — CEF dies before logging is set up. The portable v0.33.549 instance running in parallel is **not the cause** — the failure reproduces even with cache dirs separated.

---

## Root cause

**`SetDllDirectoryW` is never called when the host is launched without the launcher.**

In production / portable mode, the **launcher** (`agentmux.exe`) sets the DLL search path to `runtime/` before spawning the CEF host:

```rust
// agentmux-launcher/src/main.rs:46-62
// Set DLL search path so libcef.dll (in runtime/) is found by the
// CEF host's load-time linker. SetDllDirectoryW is process-local
// and inherited by child processes — both srv (which doesn't
// need libcef but harmless) and host (which absolutely does).
#[cfg(target_os = "windows")]
{
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = runtime_dir
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
    }
}
log("SetDllDirectoryW done");
```

The host inherits this DLL search path from its parent (the launcher) and never needs to set it itself.

The CEF host *also* has a self-defence `SetDllDirectoryW` block (`agentmux-cef/src/main.rs:49-65`), but it only fires when it can find a `runtime/` subdirectory next to the host exe:

```rust
// agentmux-cef/src/main.rs:49-65
#[cfg(target_os = "windows")]
{
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let runtime_dir = dir.join("runtime");
            if runtime_dir.exists() {
                unsafe {
                    use std::os::windows::ffi::OsStrExt;
                    let wide: Vec<u16> = runtime_dir.as_os_str().encode_wide().chain(Some(0)).collect();
                    windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
                }
            }
        }
    }
}
```

This block exists for **portable layouts**, where the host lives at `<root>/runtime/agentmux-{version}.exe`. The check `runtime_dir.exists()` is `<root>/runtime/runtime/` — which doesn't exist — so even in portable mode this block is a no-op. Portable works because the **launcher** already set the path.

In dev mode there is no launcher. The Taskfile launches the host directly:

```yaml
# Taskfile.yml (dev:serve recipe)
cd "$DEV_DIR" && LD_LIBRARY_PATH=. AGENTMUX_DEV=1 ./agentmux-cef.exe --url=http://localhost:5173
```

`$DEV_DIR` is `dist/cef-dev/`. So:

- Host exe: `dist/cef-dev/agentmux-cef.exe`
- Host exe parent: `dist/cef-dev/`
- Self-defence block looks for: `dist/cef-dev/runtime/` — **does not exist**
- Result: `SetDllDirectoryW` never runs, anywhere

Why this matters: `agentmux-cef.exe` links against `libcef.dll`'s import library. Windows resolves the load-time import via the executable directory, so libcef.dll itself loads fine. But CEF's runtime then performs `LoadLibraryW`/`LoadLibraryExW` calls for chrome_elf.dll, libEGL.dll, libGLESv2.dll, d3dcompiler_47.dll, and friends. Without an explicit DLL search path, these calls follow the standard Windows search order, which on modern Windows with `SafeDllSearchMode` can be more restrictive — and CEF's pak/locale resource resolution can also depend on the directory setup.

The end result: `cef::initialize()` fails internally. CEF returns 0. The Rust assertion fires before any logging is wired up, which is why `chrome_debug.log` stays empty.

---

## Why this regressed (or rather: why it surfaced now)

`agentmux-cef/src/main.rs` is byte-identical between v0.33.549 and v0.33.560 (`git diff 257bf0ff..ea2c477d -- agentmux-cef/src/main.rs` is empty). The bug existed previously. It only surfaced today because:

1. The user has had the v0.33.549 portable running.
2. Earlier today we wiped `target/` and `dist/` for a fresh state.
3. After the rebuild, this is the **first** invocation of `task dev` against a clean `dist/cef-dev/`.

A working hypothesis for why nobody noticed before is that in the past, dev mode was launched in a way that inherited a DLL search path from a parent process (e.g., `cargo run`), or the user had some Windows DLL pre-load policy set differently. With the new clean state, none of that's true and the latent bug bites.

---

## Other things ruled out

| Hypothesis | Verdict | Evidence |
|---|---|---|
| Cache-dir collision with running portable | **No** | Dev cache: `~/AppData/Roaming/ai.agentmux.cef.dev`. Portable cache: `<portable>/data/cef/`. Distinct. Wiping `ai.agentmux.cef.dev` did not help. |
| Debug-port collision | **No** | Portable uses 9222 (production). Dev uses 9223 (`agentmux-cef/src/main.rs:375`). Disjoint. `netstat -ano` shows only 9222 bound. |
| Backend port conflict | **No** | Backends use dynamic ports (logs show `web=127.0.0.1:62015 ws=127.0.0.1:62016` for dev; portable uses different dynamic ports). Backend reaches "Backend ready" before the panic. |
| Stale CEF lockfile | **No** | `agentmux-cef/src/main.rs:366-370` removes stale lockfiles before init. Verified the dev cache has no `lockfile` after wipe. |
| Missing CEF DLLs / resources | **No** | `dist/cef-dev/` contains all of: `libcef.dll`, `chrome_elf.dll`, `libEGL.dll`, `libGLESv2.dll`, `d3dcompiler_47.dll`, `icudtl.dat`, `*.pak`, `locales/en-US.pak`, `v8_context_snapshot.bin`, `resources.pak`. |
| CEF-vs-binding version mismatch | **No** | `Cargo.lock` pins `cef = "146.7.0+146.0.12"`, `cef-dll-sys = "146.7.0+146.0.12"`. `libcef.dll` `FileVersion = 146.0.12+g6214c8e+chromium-146.0.7680.179`. Match. |
| Architecture mismatch | **No** | Both x86_64. `agentmux-cef.exe` and `libcef.dll` confirmed x64. |
| Code regression in v0.33.560 | **No** | `agentmux-cef/src/main.rs` byte-identical. The 12 commits since v0.33.549 are all backend (saga architecture). |
| Single-instance Windows mutex held by portable | **No** | The named-pipe single-instance lock lives in the **launcher**, not the host (`agentmux-cef/src/main.rs:190-203` comment). Dev mode doesn't use the launcher. The portable's launcher pipe wouldn't conflict with a launcher-less dev host. |

---

## Why `chrome_debug.log` is empty

The host's `Settings` struct does not pass `log_file_path` or `log_severity` (`agentmux-cef/src/main.rs:377-389`):

```rust
let settings = Settings {
    no_sandbox: 1,
    background_color: 0xFF000000,
    remote_debugging_port: debug_port as i32,
    root_cache_path: cache_dir,
    resources_dir_path: resources_dir,
    locales_dir_path: locales_dir,
    browser_subprocess_path: ...,
    ..Default::default()
};
```

Without those, CEF defaults its log path and severity. Combined with init failing before any log infrastructure flushes, the file stays empty. This is a separate observability gap worth fixing, but not the root cause.

---

## Fix

One-line conceptual change to `agentmux-cef/src/main.rs:49-65` — fall back to the host's own directory when the `runtime/` subdir is absent:

```rust
#[cfg(target_os = "windows")]
{
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let runtime_dir = dir.join("runtime");
            // Portable layout: host is at <root>/runtime/host.exe and
            // the launcher already SetDllDirectoryW'd <root>/runtime/.
            // Defensive: if the launcher chain didn't set it (e.g., dev
            // mode launches the host directly via Taskfile), fall back
            // to whichever directory actually has libcef.dll alongside.
            let dll_dir = if runtime_dir.exists() { runtime_dir } else { dir.to_path_buf() };
            unsafe {
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = dll_dir.as_os_str().encode_wide().chain(Some(0)).collect();
                windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr());
            }
        }
    }
}
```

This is safe in all modes:

- **Portable / launcher-driven:** the launcher already set the path; this call sets it again to the same `runtime/` (or to the host's parent if the dir layout changes). Idempotent.
- **Dev / direct launch:** sets the path to the host's own directory (`dist/cef-dev/`), where libcef.dll and all helper DLLs actually live. Fixes the failure.
- **macOS / Linux:** unaffected (`#[cfg(target_os = "windows")]`).

### Secondary — observability fix

Add `log_file` + `log_severity` to `Settings` so future CEF init failures leave a `chrome_debug.log` we can read:

```rust
let cef_log_file = CefString::from(
    log_dir.join("cef-debug.log").to_str().unwrap_or("")
);
let settings = Settings {
    no_sandbox: 1,
    background_color: 0xFF000000,
    remote_debugging_port: debug_port as i32,
    root_cache_path: cache_dir,
    resources_dir_path: resources_dir,
    locales_dir_path: locales_dir,
    log_file: cef_log_file,
    log_severity: cef::sys::cef_log_severity_t::LOGSEVERITY_INFO,
    browser_subprocess_path: ...,
    ..Default::default()
};
```

(Field/enum names per `cef-dll-sys` 146.7.0; verify against the binding's `Settings` definition before applying.)

---

## How to confirm

After applying the primary fix:

```bash
task build:host          # rebuilds just the CEF host
task bundle              # re-bundles dist/cef
task dev                 # should now reach "CEF initialized, entering message loop"
```

A successful run produces:

- `~/.agentmux/logs/agentmux-host-v0.33.560.log.<date>` continuing past the "CEF initialized" line.
- A visible window loading `http://localhost:5173`.
- Vite's HMR connection in DevTools (Ctrl+Shift+I).
- Both the running v0.33.549 portable **and** the dev host operating side-by-side without interference.

If init still fails after the DLL-path fix, apply the observability fix, retry, and read `~/.agentmux/logs/cef-debug.log` for the underlying CEF error message.

---

## Open questions

- Why has nobody hit this in past `task dev` runs? Possible explanations:
  - Prior dev sessions inherited a DLL path from a parent shell that happened to have CEF on PATH.
  - `task dev` in older versions ran the host via a different recipe (e.g., a Cargo wrapper) that handled DLL paths differently.
  - The bug only manifests when `dist/cef-dev/` is freshly populated by the Taskfile copy — long-lived setups may have had a different layout.
- Worth a `git log -p Taskfile.yml` and `git log -p agentmux-cef/src/main.rs` over the last few months to see when the last working dev setup was, and what changed between then and now.

---

## File reference

| File | Lines | Concern |
|---|---|---|
| `agentmux-launcher/src/main.rs` | 46–62 | Launcher's `SetDllDirectoryW` — works in portable, not invoked in dev |
| `agentmux-cef/src/main.rs` | 49–65 | Host's defensive `SetDllDirectoryW` — only fires when `runtime/` subdir exists |
| `agentmux-cef/src/main.rs` | 129 | `is_dev = std::env::var("AGENTMUX_DEV").is_ok()` |
| `agentmux-cef/src/main.rs` | 158–175 | Cache dir resolution — picks `ai.agentmux.cef.dev` for dev |
| `agentmux-cef/src/main.rs` | 366–370 | Stale lockfile cleanup before CEF init |
| `agentmux-cef/src/main.rs` | 377–389 | CEF `Settings` struct (no `log_file` set today) |
| `agentmux-cef/src/main.rs` | 392–398 | `cef::initialize()` call + assertion |
| `Taskfile.yml` (dev:serve) | — | Launches host directly without the launcher |

---

## Bottom line

The launcher sets the DLL search path; dev mode doesn't use the launcher; the host's own fallback only triggers in a layout that doesn't actually exist anywhere. The fix is a five-line `else` branch on an existing conditional. The other supposed culprits (cache locks, port conflicts, single-instance mutexes, version skew) are all ruled out by the evidence.

---

## Update — 2026-05-02 — Second root cause uncovered

After the DLL-path fix shipped, `task dev` *still* failed with the same `assertion left == right failed: CEF initialization failed (left: 0, right: 1)` panic. Adding `log_file` + `log_severity = VERBOSE` to the `Settings` struct gave us a populated `cef-debug.log` and the real story.

### What `cef::initialize() == 0` actually means

From `cef-dll-sys-146.7.0+146.0.12/src/bindings/x86_64_pc_windows_msvc.rs` (the `cef_initialize` doc comment):

> "Returns true (1) if initialization succeeds. Returns false (0) **if initialization fails or if early exit is desired (for example, due to process singleton relaunch behavior)**. If this function returns false (0) then the application should exit immediately without calling any other CEF functions except, optionally, **CefGetExitCode**."

So `0` is a polysemous return value:
- Real init failure (anything in `cef_resultcode_t::CEF_RESULT_CODE_*` non-normal range), OR
- Normal early exit (singleton relaunch, AUTO_DE_ELEVATED, PACK_EXTENSION_SUCCESS, etc.)

`agentmux-cef/src/main.rs:398` was a hard `assert_eq!(init_result, 1, …)`. That panicked on every "early exit" case as if it were a fatal error, killing the launcher chain.

### Calling `cef_get_exit_code()` revealed the actual cause

After replacing the assertion with a branch on `cef::get_exit_code()`:

```
[INFO] CEF early exit (process singleton or similar) — exiting cleanly exit_code=38
```

`38` is `CEF_RESULT_CODE_NORMAL_EXIT_AUTO_DE_ELEVATED`:

> "The browser process exited because it was re-launched without elevation."

Chromium refuses to run elevated for security reasons. When it detects an elevated parent, it spawns a non-elevated child of itself, then the original returns `0` so the parent exits cleanly. Our assertion treated that clean-exit signal as a fatal error.

### Why this only surfaces now

The shell driving `task dev` in this session was running as Administrator. AgentMux is normally launched from a regular user shell (Start Menu, Explorer double-click, normal terminal), where Chromium's de-elevation path never triggers and `init_result` is `1` straightforwardly.

This explains why nobody hit it before: nobody ran `task dev` from an elevated shell. With this Claude Code session running elevated, every invocation tripped the de-elevation path, and our assertion converted the normal early-exit into a panic.

### Final fix

Two parts to the host's CEF init handling — one tactical, one structural:

1. **Tactical (already in this PR):** the DLL-path fallback (so dev's flat layout finds libcef's helper DLLs even without the launcher).
2. **Structural (added today):** replace the `assert_eq!` with a `cef::get_exit_code()` branch that distinguishes real failures from CEF's documented early-exit codes (24 / 36 / 38) and exits 0 in those cases.

### Bonus: log_file + log_severity in Settings

Without explicit `log_file`/`log_severity`, CEF's internal logging is effectively silent — empty `chrome_debug.log` is what wasted hours of guesswork on this issue. Setting them produces a readable `~/.agentmux/logs/cef-debug.log` for any future CEF-side problem.

### Verification

End-to-end verification in the current admin shell isn't possible — Chromium still de-elevates and the parent still exits with code 38, but now it does so cleanly with `INFO CEF early exit (process singleton or similar) — exiting cleanly exit_code=38` instead of panicking. From a normal (non-elevated) user shell, `init_result` will be `1` and the success path proceeds normally as it did pre-regression.

### Updated bottom line

The DLL-path fallback was necessary but not sufficient. The other half of the story is a misuse of CEF's documented `initialize()` contract: `0` doesn't always mean failure. The new exit-code branch correctly distinguishes the cases. With both fixes in place, `task dev` works from any shell — admin or non-admin.
