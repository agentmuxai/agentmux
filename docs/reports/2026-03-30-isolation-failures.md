# AgentMux Isolation Failures — 2026-03-30

**Severity:** High — blocks development workflow when any AgentMux instance is running
**Principle violated:** Each AgentMux instance (dev, portable, different versions) must be 100% isolated. No instance should block builds, bundling, or launching of any other instance.

---

## Failure 1: `dist/cef/` is a shared mutable staging directory

**Symptom:** `task cef:bundle` fails with "The process cannot access the file because it is being used by another process" on `dist/cef/libcef.dll`.

**Root cause:** The `cef:bundle:windows` task copies CEF runtime DLLs (`libcef.dll`, `chrome_elf.dll`, etc.) from the cargo build output into `dist/cef/`. The `cef:dev:serve` task then runs `dist/cef/agentmux-cef.exe` directly. While that process is running, `libcef.dll` is memory-mapped by the OS and cannot be overwritten.

Even if the *running* instance is a portable from `~/Desktop/agentmux-cef-0.33.4-x64-portable/`, a *previous* dev session may have left `dist/cef/agentmux-cef.exe` running. And the bundle task unconditionally tries to overwrite `dist/cef/libcef.dll`.

**Impact:**
- Cannot rebuild + relaunch during a dev session without manually killing the CEF process
- A portable instance on Desktop with locked DLLs at the same paths shouldn't matter, but the bundle task doesn't check

**Fix requirements:**
1. `cef:bundle` should copy to a **versioned** staging dir: `dist/cef-{version}/` instead of `dist/cef/`
2. `cef:dev:serve` should launch from the versioned dir
3. Alternatively, bundle into a temp dir and rename atomically — if the target is locked, leave the old one and use a new path
4. The dev CEF host should use a unique data dir (already does: `ai.agentmux.cef.v0-33-N`) but the **binary staging dir** is not versioned

---

## Failure 2: CEF initialization fails when DLLs are stale/mismatched

**Symptom:** `assertion failed: CEF initialization failed` — `cef_initialize()` returns 0.

**Root cause:** When `cef:bundle` fails mid-copy (e.g., `libcef.dll` locked), the `dist/cef/` directory has a mix of old and new files. The new `agentmux-cef.exe` (v0.33.5) tries to load the old `libcef.dll` (v0.33.4 build) and CEF init fails due to version mismatch or corrupted state.

**Impact:** Silent failure — no clear error message about which DLL is stale. User sees a cryptic assertion failure.

**Fix requirements:**
1. `cef:dev:serve` should verify that `agentmux-cef.exe` and `libcef.dll` timestamps match (or check embedded versions)
2. If `cef:bundle` fails, `cef:dev:serve` must not launch — it should fail fast with a clear message
3. Consider a version stamp file (`dist/cef/.version`) written at the end of a successful bundle, checked before launch

---

## Failure 3: `cef-dll-sys` build output is not reusable across version bumps

**Symptom:** Every `bump patch` invalidates the cargo build hash for `cef-dll-sys`, triggering a 155MB CEF SDK re-download and re-extraction. Windows Defender scans the extraction and causes intermittent "Access is denied" errors.

**Root cause:** `cef-dll-sys` build script uses `$OUT_DIR` (which includes a cargo fingerprint hash) as the extraction target. The hash changes whenever `Cargo.toml` version changes. The crate has no mechanism to reuse a previous extraction.

**Mitigation applied:** Set `CEF_PATH` in `.cargo/config.toml` to a stable path (`C:/Systems/cef-runtime/cef_146.0.9_windows_x86_64`). This skips the download entirely.

**Remaining risk:** The `CEF_PATH` directory must contain `archive.json`, `libcef_dll/`, `include/`, `cmake/`, `CMakeLists.txt` — all of which were manually assembled. If someone clones the repo fresh, they'll hit the original download issue unless `CEF_PATH` is documented.

**Fix requirements:**
1. Document `CEF_PATH` setup in `BUILD.md` or `CLAUDE.md`
2. Add a `task cef:setup` that automates the stable CEF extraction
3. Consider committing `.cargo/config.toml` with `CEF_PATH` pointing to a project-relative path (e.g., `.cef-sdk/`) and add a one-time download task

---

## Failure 4: `task dev` doesn't gate launch on successful build

**Symptom:** `task dev` calls `cef:build` → `cef:bundle` → `cef:dev:serve`. If `cef:bundle` fails (DLL locked), it still proceeds to `cef:dev:serve` which launches the stale binary.

**Root cause:** The Taskfile `cmds` list runs sequentially but the error from `cef:bundle:windows` is a `coreutils` copy error, which may not propagate as a task failure depending on the shell.

**Fix requirements:**
1. Ensure `cef:bundle:windows` exits non-zero on copy failure (verify the PowerShell script uses `throw` or `$ErrorActionPreference = 'Stop'`)
2. Add an explicit check in `cef:dev:serve` that `dist/cef/libcef.dll` exists and is not locked
3. If locked, print a clear message: "Another AgentMux instance is using dist/cef/. Close it or use a different dist path."

---

## Failure 5: Multiple CEF processes share the same `dist/cef/` working directory

**Symptom:** Opening a second dev window (`cef:run`) while one is already running works because they load DLLs into memory, but the *next* bundle/build cycle can't update the shared directory.

**Root cause:** CEF loads `libcef.dll` from the exe's directory. All dev instances point to the same `dist/cef/`. Windows locks DLLs that are memory-mapped by any process.

**Fix requirements:**
1. Each dev launch should work from a **copy** or **symlinked** versioned directory
2. Or: `cef:dev:serve` should copy the bundle to a temp dir (`dist/cef-dev-{pid}/`) and launch from there, so `dist/cef/` is never locked
3. Portable builds are already isolated (each ZIP extracts to its own folder) — dev mode needs the same treatment

---

## Summary: What "100% isolated" means

| Resource | Current State | Required State |
|----------|--------------|----------------|
| User data dirs | Isolated (`ai.agentmux.cef.v0-33-N`) | OK |
| WebView2 UDF | Isolated (versioned identifier) | OK |
| Build output (`target/`) | Shared but not locked by portable | OK |
| CEF SDK (`cef-dll-sys` extraction) | Fixed via `CEF_PATH` | OK |
| Dev binary staging (`dist/cef/`) | **Shared, lockable** | Must be versioned or per-session |
| Portable binary dir | Isolated (each ZIP) | OK |
| `libcef.dll` file lock | **Blocks bundle task** | Must not block other instances |
| Vite dev server (port 5173) | **Shared port** | Should use dynamic or versioned port |

## Priority order for fixes

1. **Versioned `dist/cef-{version}/`** — unblocks dev workflow immediately
2. **Gate `cef:dev:serve` on successful bundle** — prevents stale launches
3. **`task cef:setup`** — automates CEF SDK setup for fresh clones
4. **Per-session dev directory** — full isolation for parallel dev instances
