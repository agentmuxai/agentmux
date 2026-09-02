# Cleanup Spec: Remove Legacy Remnants

**Date:** 2026-03-03
**Goal:** Remove all Go, Electron, and stale Wave Terminal remnants. The project is 100% Rust + TypeScript + Tauri v2 + Task.

---

## Category 1: CI/CD — Go References (CRITICAL)

The CI workflow is completely broken for the current Rust architecture.

### `.github/workflows/tauri-build.yml`
| Line(s) | Issue | Fix |
|---------|-------|-----|
| 41-45 | `actions/setup-go@v5` with Go 1.23 | **Delete** entire step |
| 67-98 | "Build Go backend binaries" step: `CGO_ENABLED`, `GOOS`, `GOARCH`, `go build ./cmd/server/main-server.go`, `go build ./cmd/wsh/main-wsh.go` | **Replace** with Rust `cargo build` step (see RELEASE_v0.31.20.md) |
| 88,94 | `-ldflags "-X main.WaveVersion=$VERSION"` | Gone with Go removal |
| 107-115 | "Copy sidecar binaries" uses Go naming | **Rewrite** for Rust target triple naming |
| 127 | `tauriScript: npm run tauri:build` | Change to `tauriScript: npx tauri` |

**Action:** Full workflow rewrite per `specs/RELEASE_v0.31.20.md`.

---

## Category 2: Electron Remnants (HIGH)

### `tsconfig.json` — line 2
```json
"include": ["frontend/**/*", "emain/**/*"]
```
`emain/` was the Electron main process directory. It no longer exists.
**Fix:** Remove `"emain/**/*"` from includes.

### `package.json` — line 15
```json
"main": "./dist/main/index.js"
```
This is the Electron entry point. Tauri doesn't use this.
**Fix:** Remove the `"main"` field entirely.

### `agentmuxsrv-rs/src/backend/eventbus.rs` — lines 18-20
```rust
pub const WS_EVENT_ELECTRON_NEW_WINDOW: &str = "electron:newwindow";
pub const WS_EVENT_ELECTRON_CLOSE_WINDOW: &str = "electron:closewindow";
pub const WS_EVENT_ELECTRON_UPDATE_ACTIVE_TAB: &str = "electron:updateactivetab";
```
**Fix:** Rename to `tauri:newwindow`, `tauri:closewindow`, `tauri:updateactivetab`. Search for all usages in both Rust and frontend code and update consistently.

### `agentmuxsrv-rs/src/backend/rpc/router.rs` — line 24
```rust
pub const ELECTRON_ROUTE: &str = "electron";
```
**Fix:** Rename to `"tauri"`. Update all references in router logic + frontend `tabrpcclient.ts`.

### `src-tauri/src/sidecar.rs` — line 380, 387
```rust
/// Handle WAVESRV-EVENT messages from the backend.
let _ = window.emit("wavesrv-event", event_data.to_string());
```
The comment and event name reference the old `wavesrv` naming.
**Fix:** Rename event to `"agentmuxsrv-event"` and update the comment to `/// Handle AGENTMUXSRV-EVENT messages from the backend.` Search frontend for any listener using this event name and update consistently.

### `eslint.config.js` — lines 11-18
```js
overrides: [
    {
        files: ["emain/emain.ts", "electron.vite.config.ts"],
        env: { node: true },
    },
],
```
References deleted Electron files.
**Fix:** Remove the entire `overrides` block.

### `vitest.config.ts` — lines 2, 4-5 (BROKEN)
```ts
import electronViteConfig from "./electron.vite.config";
// ...
electronViteConfig.renderer as UserConfig,
```
Imports from `electron.vite.config` which no longer exists. This file is **broken** and will fail if vitest is run.
**Fix:** Rewrite to import from `vite.config.tauri.ts` instead, or create a standalone vitest config without merging from a non-existent file.

---

## Category 3: `make/` Build Directory (HIGH)

The old Electron/Go build system used `make/` for artifacts. Tauri uses `src-tauri/target/` and `dist/`.

### `Taskfile.yml` — line 337
```yaml
ORIGIN: "make/"
```
The `artifacts:upload` task uploads from `make/` which no longer contains build output.
**Fix:** Change to `"dist/"` or `"src-tauri/target/release/bundle/"`.

### `Taskfile.yml` — lines 448-450
```yaml
clean:
    desc: clean make/dist directories
    cmds:
        - cmd: '{{.RMRF}} "make"'
```
**Fix:** Remove `make` from clean description and command. Keep `dist` cleanup.

### `.gitignore`
Likely has `make/` entries.
**Fix:** Remove `make/` entries, ensure `src-tauri/target/` is ignored.

---

## Category 4: Bug Report Template (HIGH)

### `.github/ISSUE_TEMPLATE/bug-report.yml` — lines 39-67
All user-facing text says "Wave":
- "version of Wave you're running"
- "Wave -> About Wave Terminal"
- Label: "Wave Version"
- "where you are running Wave" (3 occurrences)

**Fix:** Replace all `Wave`/`Wave Terminal` with `AgentMux`.

---

## Category 5: Rust Source — `wave*` Naming (MEDIUM)

These are internal API/protocol constants. Not user-facing, but confusing for contributors.

### Source file names to rename:
| Current | Proposed |
|---------|----------|
| `agentmuxsrv-rs/src/backend/wavebase.rs` | Keep (migration code, references `.waveterm` dir) |
| `agentmuxsrv-rs/src/backend/wavefileutil.rs` | `fileutil.rs` |
| `agentmuxsrv-rs/src/backend/waveapp.rs` | `app.rs` or `apprunner.rs` |

### Constants to rename in `rpc_types.rs`:
| Current | Proposed | Line |
|---------|----------|------|
| `COMMAND_WAVE_INFO` = `"waveinfo"` | Keep string value (wire protocol), rename const to `COMMAND_APP_INFO` | 204 |
| `COMMAND_WAVE_AI_ENABLE_TELEMETRY` | `COMMAND_AI_ENABLE_TELEMETRY` | 241 |
| `COMMAND_GET_WAVE_AI_CHAT` | `COMMAND_GET_AI_CHAT` | 242 |
| `COMMAND_GET_WAVE_AI_RATE_LIMIT` | `COMMAND_GET_AI_RATE_LIMIT` | 243 |
| `COMMAND_WAVE_AI_TOOL_APPROVE` | `COMMAND_AI_TOOL_APPROVE` | 244 |
| `COMMAND_WAVE_AI_ADD_CONTEXT` | `COMMAND_AI_ADD_CONTEXT` | 245 |

### Other Rust constants:
| File | Current | Proposed |
|------|---------|----------|
| `rpc/router.rs:21` | `DEFAULT_ROUTE = "wavesrv"` | Keep string (wire protocol compat), rename const |
| `wavefileutil.rs:17` | `WAVE_FILE_PATH_PATTERN = "wavefile://"` | Keep string (protocol URL), rename const to `FILE_PATH_PATTERN` |
| `vdom.rs:24` | `WAVE_TEXT_TAG = "wave:text"` | Keep string (VDOM protocol), rename const |
| `vdom.rs:27` | `WAVE_NULL_TAG = "wave:null"` | Keep string (VDOM protocol), rename const |

**Important:** The string VALUES of these constants are wire protocol identifiers shared between backend and frontend. Changing them would break compatibility. Only rename the Rust CONST NAMES for readability.

### `wsh-rs/Cargo.toml` — line 5
```toml
description = "Wave Shell Helper - CLI tool to control AgentMux"
```
**Fix:** Change to `"AgentMux Shell Helper - CLI tool to control AgentMux"` or just `"Shell integration CLI for AgentMux"`.

---

## Category 6: Frontend — `wave.ts` References (LOW)

These reference the file `frontend/wave.ts` which is the app entry point. Not a branding issue — it's an internal filename.

| File | Line | Content |
|------|------|---------|
| `frontend/tauri-bootstrap.ts` | 137-143 | `"Loading main application (wave.ts)..."`, `"Failed to load wave.ts:"` |
| `frontend/tauri-init.ts` | 15 | `"This MUST be awaited before importing wave.ts"` |
| `frontend/app/store/global.ts` | 169 | `"initialized in wave.ts"` |

**Decision:** Optional rename. `wave.ts` could become `app.ts` or `agentmux.ts`, but it's low priority since it's an internal module. The comments are accurate references to the filename.

---

## Category 7: `docs/` Build Artifacts (MEDIUM)

### `docs/build/` — 59+ files with `waveterm` references
### `docs/.docusaurus/` — cached build config with `waveterm`

These are Docusaurus build artifacts that should NOT be in the repo.

**Fix:** Delete `docs/build/` and `docs/.docusaurus/` entirely. Verify `.gitignore` covers them. If they're tracked, unstage them.

### `package-lock.json` — lines 147, 160
```json
"name": "waveterm-docs"
"@waveterm/docusaurus-og": "https://codeload.github.com/wavetermdev/..."
```
These are in the `docs` workspace. The docs package references upstream Wave plugins.

**Fix:** If docs workspace is being kept, update package name. If not needed for v0.31.20 release, consider removing the `docs` workspace from `package.json` workspaces array.

---

## Category 8: Stale Specs & Docs (LOW)

### Specs referencing old architecture:
| File | Contains |
|------|----------|
| `specs/go-to-rust-backend-port.md` | Go→Rust migration plan (completed) |
| `specs/archive/phase14-electron-removal-complete.md` | Electron removal report |
| `specs/archive/tauri-migration-complete.md` | Migration report |
| `specs/tabbar-enhancements.md` | References `~/.waveterm/config/widgets.json` (lines 279, 411) |
| `specs/rebrand.md` | Rebrand tracking doc with old names |

**Fix:** Move completed migration specs to `specs/archive/`. Fix `~/.waveterm` paths in active specs to `~/.agentmux`.

### `SECURITY_CLEANUP_REPORT_2026-03-03.md`
This is a private internal report. Should not be in the public repo.
**Fix:** Delete from repo (already archived to private corporate repo).

---

## Category 9: Shell Integration Scripts (LOW)

### `agentmuxsrv-rs/src/backend/shellintegration/fish.fish`
Contains `wave.` references in OSC escape sequences.
**Fix:** Audit all shell integration scripts (bash, zsh, fish, pwsh) for Wave references. These are part of the wsh protocol — string values may need to stay for backward compat, but comments should be updated.

---

## Execution Order

### Phase 1 — Breaking/CI (do first)
1. Rewrite `.github/workflows/tauri-build.yml` (Go → Rust)
2. Fix `.github/ISSUE_TEMPLATE/bug-report.yml` (Wave → AgentMux)
3. Delete `SECURITY_CLEANUP_REPORT_2026-03-03.md` from repo

### Phase 2 — Config/Build
4. Fix `tsconfig.json` (remove `emain/**/*`)
5. Fix `package.json` (remove `"main"` field)
6. Fix `Taskfile.yml` (`make/` → `dist/`, clean task)
7. Fix `.gitignore` if needed
8. Delete `docs/build/` and `docs/.docusaurus/` if tracked

### Phase 3 — Rust Rename (careful, cross-cutting)
9. Rename `ELECTRON_ROUTE` → `TAURI_ROUTE` + `electron:*` event constants → `tauri:*`
10. Rename `wave*` Rust const names (keep string values)
11. Rename `wavefileutil.rs` → `fileutil.rs`, `waveapp.rs` → `apprunner.rs`
12. Fix `wsh-rs/Cargo.toml` description

### Phase 4 — Cleanup
13. Fix `specs/tabbar-enhancements.md` waveterm paths
14. Move completed specs to archive
15. Remove `docs` workspace if not needed
16. Optional: rename `frontend/wave.ts` → `frontend/app.ts`

---

## Checklist

- [ ] `.github/workflows/tauri-build.yml` — full rewrite
- [ ] `.github/ISSUE_TEMPLATE/bug-report.yml` — Wave → AgentMux
- [ ] `SECURITY_CLEANUP_REPORT_2026-03-03.md` — delete from repo
- [ ] `tsconfig.json` — remove `emain/**/*`
- [ ] `package.json` — remove `"main"` field
- [ ] `Taskfile.yml` — `make/` → `dist/`, clean task
- [ ] `eventbus.rs` — `electron:*` → `tauri:*`
- [ ] `router.rs` — `ELECTRON_ROUTE` → `TAURI_ROUTE`
- [ ] `sidecar.rs` — `wavesrv-event` → `agentmuxsrv-event`
- [ ] `rpc_types.rs` — rename `COMMAND_WAVE_*` consts
- [ ] `wavefileutil.rs` → `fileutil.rs`
- [ ] `waveapp.rs` → `apprunner.rs`
- [ ] `wsh-rs/Cargo.toml` — fix description
- [ ] `bug-report.yml` — all Wave → AgentMux
- [ ] `docs/build/`, `docs/.docusaurus/` — delete if tracked
- [ ] `specs/tabbar-enhancements.md` — fix `.waveterm` paths
- [ ] Verify build after all changes
