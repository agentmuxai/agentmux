# AgentMux Full Codebase Audit — 2026-04-03

**Status:** Historical — a point-in-time snapshot, not current status.

> **Staleness note (2026-08-03):** confirmed stale, not just old. `src-tauri/` (issue #1) is gone — no longer in the workspace or referenced by the build. The "dual state management" finding (issue #2) is also out of date — the frontend is now essentially SolidJS-only. Read this for historical context on the Tauri→CEF migration, not as a current-state audit; see `docs/reports/` for more recent repo-health passes.

## Executive Summary

The codebase is **solid but carries significant technical debt** from the Tauri→CEF migration and the Go→Rust port. Three systemic issues dominate:

1. **Tauri is still in the workspace** — `src-tauri/` is 31MB of dead code still compiled by `cargo check`, referenced by `.bump.json`, and the `package` task literally calls `npx tauri build` instead of CEF
2. **Dual state management** — Frontend runs Jotai atoms, a globalStore shim, AND SolidJS signals simultaneously across 11+ files
3. **God modules** — `wcore.rs` (1419 lines), `wconfig.rs` (1624 lines), `shell.rs` (1325 lines), `global.ts` (917 lines) each bundle unrelated concerns

---

## Part 1: Should We Delete src-tauri/?

### YES — Immediately

**It is completely dead:**
- Not imported by any active crate (agentmux-cef, agentmux-srv, agentmux-wsh)
- Not built by any active task (`task dev`, `task cef:package:portable`)
- Last real code change: March 7, 2026
- Only touched since by automated version bumps via `.bump.json`

**It actively causes problems:**
- Still in workspace `Cargo.toml` members → `cargo check` compiles it
- `.bump.json` bumps `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` on every version bump — wasted git noise
- The `package`, `package:macos`, `package:portable:linux` Taskfile tasks call `npx tauri build` — **they're broken and would produce wrong output if anyone ran them**

**What to keep:**
- `frontend/tauri-*.ts` files — runtime compatibility shim (checks `__TAURI_INTERNALS__` at runtime, no-ops on CEF)
- `@tauri-apps/*` npm deps — used by the frontend shim and test infrastructure
- Test helpers (`wdio.conf.cjs`, `test/helpers/tauri-helpers.js`) — still reference Tauri mocks

**Deletion order:**
1. Remove `src-tauri` from `Cargo.toml` workspace members
2. Remove `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` targets from `.bump.json`
3. Delete deprecated Taskfile tasks: `dev:tauri`, `tauri:dev`, `tauri:build`, `tauri:copy-sidecars`, `start`, `quickdev`, `sync:dev:binaries:*`
4. Fix `package`/`package:macos`/`package:portable:linux` to use CEF build pipeline
5. Delete scripts: `sync-version.sh`, `update-tauri.sh`, `verify-tauri-versions.sh`, `build-release.ps1`
6. Delete entire `src-tauri/` directory
7. Clean `.gitignore` entries for `src-tauri/`
8. Update BUILD.md, CONTRIBUTING.md

**Files outside src-tauri/ that reference it (must update):**
- `Cargo.toml` (workspace members)
- `.bump.json` (version targets)
- `Taskfile.yml` (52 lines in 7+ tasks)
- `vite.config.tauri.ts` (watch ignore — harmless but confusing name)
- `.gitignore`
- `scripts/sync-version.sh`, `scripts/update-tauri.sh`, `scripts/verify-tauri-versions.sh`
- `scripts/benchmarks/measure-performance.{ps1,sh}`, `scripts/build-appimage.sh`, `scripts/package-msix.ps1`
- `wdio.conf.cjs` (test config)
- `BUILD.md`, `CONTRIBUTING.md`

---

## Part 2: Frontend Audit

### CRITICAL

| Issue | File(s) | Details |
|-------|---------|---------|
| **Dead Jotai in BlockModel** | `frontend/app/block/block-model.ts:4-5, 14-32` | Still imports and uses `jotai.atom()` despite migration to SolidJS signals. Creates unmaintained dual-state system |
| **Direct Tauri imports in active code** | `frontend/app/drag/CrossWindowDragMonitor.linux.tsx:14`, `darwin.tsx:61`, `frontend/app/view/term/term.tsx:274` | Hardcoded `import { invoke } from "@tauri-apps/api/core"` instead of using platform IPC abstraction — breaks on CEF |

### HIGH

| Issue | File(s) | Details |
|-------|---------|---------|
| **God file: global.ts** | `frontend/app/store/global.ts` (917 lines) | 104+ exports mixing atoms, services, utilities, counters, notifications. Split into focused modules |
| **globalStore shim used in 11 files** | `blockframe.tsx`, `termwrap.ts`, `keymodel.ts`, `agent-model.ts`, `term.tsx`, etc. | Jotai compatibility shim obscures incomplete migration to SolidJS signals |
| **Dual state management** | Across frontend | Code uses SolidJS signals + globalStore shim + raw Jotai atoms + window globals simultaneously |

### MEDIUM

| Issue | File(s) | Details |
|-------|---------|---------|
| **52 `any` types** | Throughout frontend | `global.ts:889`, `wos.ts:67,94`, `wshclient.ts:50,75`, `blockutil.tsx:56,88` — loses type safety |
| **40+ `as any` casts** | `block.tsx:41-50`, `blockframe.tsx:91,100`, `global.ts:180` | All ViewModels cast as `any` in block registry |
| **52 `(window as any)` casts** | `wave.ts` (19 occurrences), `ipc.ts`, `cef-api.ts`, `tauri-api.ts` | Should use proper type declarations for window globals |
| **Tauri-only features silently broken on CEF** | `frontend/util/notification.ts:56-71` — `sendNativeNotification()` no-ops on CEF; `frontend/wave.ts:68-106` — instance ID title no-ops on CEF | Features that work on Tauri but silently fail on CEF |
| **Dead Tauri bootstrap files** | `frontend/tauri-bootstrap.ts` (210 lines), `frontend/tauri-init.ts` (31 lines) | Conditional on `__TAURI_INTERNALS__` — never active on CEF, but add weight |
| **Leaked setInterval in TermViewModel** | `frontend/app/view/term/termViewModel.ts:32-33` | Global `setInterval(() => ..., 60_000)` never cleaned up — accumulates on re-creation |
| **Global mutable state in drag handlers** | `CrossWindowDragMonitor.linux.tsx:23`, `darwin.tsx:22-30` | Module-level `let _currentDragPayload` — concurrent drags overwrite each other |
| **Unbounded OSC buffer** | `frontend/app/view/term/termosc.ts:45-100` | No size limit on JSON parsing from terminal escape sequences |

### LOW

| Issue | File(s) | Details |
|-------|---------|---------|
| Duplicate platform detection logic | `ipc.ts:19-27`, `wave.ts:57-65` | Same `isTauri()`/`isCef()` check in multiple places |
| `setTimeout(fn, 0)` anti-pattern | 18 files | Should use `queueMicrotask()` or `requestAnimationFrame()` |
| Import path inconsistencies | Various | Mix of `@/app/store/global` and `@/store/global` |
| TODO comments without owners | `wave.ts:439,551,650` | Stale technical debt markers |
| Commented-out tests | `autotitle.test.ts:181` | Disabled but not removed |

---

## Part 3: Rust Backend Audit

### CRITICAL

| Issue | File(s) | Details |
|-------|---------|---------|
| **Blocking std::sync::Mutex in async code** | `agentmux-srv/src/backend/ai/tools.rs:210,225,249,276,293,299` | `.lock().unwrap()` on `std::sync::Mutex` inside async functions blocks the tokio executor thread. Replace with `tokio::sync::Mutex` |
| **Panic on lock poisoning** | `agentmux-cef/src/ipc.rs:191`, `main.rs:174,186`, `sidecar.rs:58,59,80,131`, `client.rs:75,130,334` | `.lock().unwrap()` on mutexes — if any thread panics while holding the lock, ALL subsequent lock attempts crash. Use `parking_lot::Mutex` (no poisoning) or handle the error |
| **Path traversal in home dir expansion** | `agentmux-srv/src/backend/wavebase.rs:250-261` | `expand_home_dir()` rejects `~user/` but accepts bare paths like `../../../root` without canonical validation |

### HIGH

| Issue | File(s) | Details |
|-------|---------|---------|
| **36+ files with blanket `#![allow(dead_code)]`** | `wavebase.rs:7`, `waveobj.rs:7`, `wavefileutil.rs:10`, and 33 more | Masks actually-unused code. Examples: `migrate_legacy_data_dir()`, `validate_wsh_supported_arch()`, entire `parse_wave_file_path()` |
| **String-based errors instead of typed enums** | `wavebase.rs:104-120`, `shell.rs`, many older modules | `Result<T, String>` loses error context. Storage layer has proper `StoreError` but legacy modules don't |
| **.expect() in CEF UI thread callbacks** | `agentmux-cef/src/client.rs:69,126,224` | `browser.expect("Browser is None")` in CEF callbacks — panicking here crashes CEF. Should return gracefully |
| **God modules** | `wconfig.rs` (1624 lines), `wcore.rs` (1419 lines), `shell.rs` (1325 lines), `wstore.rs` (1166 lines) | Bundle unrelated concerns. `wcore.rs` has workspace creation, window management, block operations, and layout logic in one file |

### MEDIUM

| Issue | File(s) | Details |
|-------|---------|---------|
| **Legacy Wave naming throughout** | `wavebase.rs`, `waveobj.rs`, `wavefileutil.rs`, `wconfig.rs`, `wcore.rs`, `wps.rs` | Go-era "Wave" prefix still everywhere. Constants like `WAVE_LOCK_FILE`, `wave.sock` coexist with `REMOTE_WAVE_HOME_DIR_NAME: &str = ".agentmux"` |
| **Blocking on async in main()** | `agentmux-cef/src/main.rs:173,182` | `runtime.block_on(ipc::start_ipc_server(...))` and `runtime.block_on(sidecar::spawn_backend(...))` — blocks UI thread during startup |
| **Mixed logging: eprintln + tracing** | Throughout | Some paths use `eprintln!`, others use `tracing::warn!`. No structured key-value logging |
| **IPC token in plaintext HTTP** | `agentmux-cef/src/main.rs:150-154` | Auth token sent in HTTP request to localhost — acceptable for local IPC but could use Unix domain sockets on Linux/macOS |
| **Unbounded channels** | `agentmux-srv/src/backend/rpc/router.rs:84-93` | `mpsc::unbounded_channel()` for RPC messages — no flow control under load |

### LOW

| Issue | File(s) | Details |
|-------|---------|---------|
| Clone abuse in merge_meta | `waveobj.rs:67-99` | Clones entire map upfront even for no-op merges |
| No graceful shutdown of watchdog | `blockcontroller/watchdog.rs` | `tokio::spawn()` with no cancellation token |
| Raw libc calls without nix crate | `agentmux-srv/src/main.rs:30-141` | Parent process monitoring uses raw `libc::` calls — `nix` crate would be safer |

### Rust Strengths (no changes needed)
- No SQL injection (parameterized queries throughout)
- No command injection (args passed as arrays, not shell-concatenated)
- No unsafe code in core logic (only in platform-specific FFI)
- Proper trait-based abstractions (`Controller`, `ConnInterface`, `RpcClient`)
- Strong typing with `Uuid`, enums, newtype patterns

---

## Part 4: Build System Audit

### Dead Taskfile Tasks
| Task | Status | Action |
|------|--------|--------|
| `dev:tauri` | Deprecated, marked in desc | Delete |
| `tauri:dev` | Unused | Delete |
| `tauri:build` | Unused | Delete |
| `tauri:copy-sidecars` | Only used by deprecated tasks | Delete |
| `start` | Unused (`cd src-tauri && cargo run`) | Delete |
| `quickdev` | Unused (`tauri dev --config ...`) | Delete |
| `sync:dev:binaries:*` | Only dep of `dev:tauri` | Delete |
| `package` | **BROKEN** — calls `npx tauri build` | Rewrite for CEF |
| `package:macos` | **BROKEN** — calls `npx tauri build` | Rewrite for CEF |
| `package:portable:linux` | **BROKEN** — calls `npx tauri build` | Rewrite for CEF |
| `artifacts:upload` | References non-existent `src-tauri/target/release/bundle/` | Delete or rewrite |

### Dead Scripts
| Script | Action |
|--------|--------|
| `scripts/sync-version.sh` | Delete (`.bump.json` handles this) |
| `scripts/update-tauri.sh` | Delete |
| `scripts/verify-tauri-versions.sh` | Delete |
| `scripts/build-release.ps1` | Delete (unused, not called by any task) |
| `scripts/build-appimage.sh` | Rewrite for CEF or delete |
| `scripts/package-msix.ps1` | Rewrite for CEF or delete |
| `scripts/benchmarks/measure-performance.{ps1,sh}` | Update paths from src-tauri to target/release |

### Stale dist/ Artifacts
| Artifact | Action |
|----------|--------|
| `dist/agentmux-0.32.106-x64-portable.zip` | Delete |
| `dist/agentmux-cef-portable/` | Delete |
| `dist/cef-release/` | Delete |
| Old wsh binaries (40+ versioned copies) | Delete all except current version |

### Configuration Issues
| File | Issue | Action |
|------|-------|--------|
| `.bump.json` | Targets `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` | Remove both |
| `vite.config.tauri.ts` | Name implies Tauri-only but CEF uses it too | Rename to `vite.config.ts` |
| `Cargo.toml` | `src-tauri` in workspace members | Remove |
| `.gitignore` | `src-tauri/target/`, `src-tauri/binaries/`, `src-tauri/gen/schemas/` | Remove |

---

## Part 5: Priority Action Plan

### P0 — Do Now (blocks other work)
1. **Delete src-tauri/** and clean all references (see Part 1 deletion order)
2. **Replace `std::sync::Mutex` with `tokio::sync::Mutex`** in `ai/tools.rs` — prevents executor thread starvation
3. **Fix `.lock().unwrap()` in CEF callbacks** — use `parking_lot::Mutex` or handle poison errors

### P1 — This Week
4. **Remove globalStore shim** — convert remaining 11 files to SolidJS signals, delete `jotaiStore.ts`
5. **Fix direct Tauri imports** in `CrossWindowDragMonitor.linux.tsx`, `darwin.tsx`, `term.tsx` — route through platform IPC abstraction
6. **Add canonical path validation** in `expand_home_dir()`
7. **Remove blanket `#![allow(dead_code)]`** — audit and delete truly dead functions
8. **Delete dead Taskfile tasks and scripts** (see tables above)

### P2 — This Sprint
9. **Split god modules**: `wcore.rs` → `workspace.rs` + `window.rs` + `block.rs` + `layout.rs`; `global.ts` → `atoms.ts` + `services.ts` + `counters.ts` + `notifications.ts`
10. **Rename legacy Wave modules**: `wavebase.rs` → `base.rs`, `waveobj.rs` → `object.rs`, etc. Update `WAVE_LOCK_FILE` → `AGENTMUX_LOCK_FILE`
11. **Define proper error types** for `wavebase`, `shell`, and other string-error modules
12. **Fix `package` Taskfile tasks** to use CEF build pipeline
13. **Clean stale dist/ artifacts**
14. **Rename `vite.config.tauri.ts`** → `vite.config.ts`

### P3 — Backlog
15. Replace `any` types (52 occurrences) with proper generics
16. Replace `(window as any)` (52 occurrences) with type declarations
17. Fix leaked `setInterval` in `TermViewModel`
18. Add bounded channels in `rpc/router.rs`
19. Use `nix` crate instead of raw `libc::` calls
20. Add structured key-value logging throughout Rust backend

---

## Metrics

| Category | Count |
|----------|-------|
| Dead Tauri code (files) | 109+ (src-tauri/) + 5 frontend + 7 scripts |
| `any` / `as any` types | 92 |
| `(window as any)` casts | 52 |
| `.lock().unwrap()` in production | 20+ |
| `#![allow(dead_code)]` blankets | 36 files |
| God modules (>1000 lines) | 4 Rust + 1 TypeScript |
| Dead Taskfile tasks | 11 |
| Dead scripts | 6 |
| Legacy "Wave" naming | 8 module files + 10+ constants |
| Estimated cleanup effort | ~40-60 hours total |
