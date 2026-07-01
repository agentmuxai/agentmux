# Large File Modularization Plan

**Date:** 2026-07-01  
**Scope:** Rust (agentmux-srv, agentmux-cef) + TypeScript/SolidJS (frontend)  
**Trigger:** Audit of files >500 lines; top offenders identified for phased extraction  
**Prior art:** See "Prior Work" section below — significant modularization has already shipped.

---

## Prior Work (already completed)

Several large-file splits have shipped since April 2026. This plan builds on them rather than repeating them.

| Spec | What shipped | Result |
|------|-------------|--------|
| `PLAN_SRV_REDUCER_MODULARIZATION_2026-05-07.md` | `reducer.rs` (4,389 lines) → `reducer/` subdir with 7 modules (block, layout, lifecycle, snapshot, tab, window, workspace) | ✅ Done — `reducer/` is 1,726 lines total, each file well-scoped |
| `websocket-modularization.md` | `websocket.rs` (2,090 lines) → handlers split into submodules (A8, PR #1554) | ✅ Done — `websocket.rs` now 1,012 lines |
| `SPEC_STORE_MODULARIZATION_2026_05_27.md` | `wstore.rs` (5,530 lines) → renamed to `store.rs` + extracted `agents.rs`, `identities.rs`, `dual_write.rs`, `memory_bundles.rs`, `content.rs`, `skills.rs`, `history.rs` | ✅ Partially done — R.0 rename + R.1 (agents.rs 1,841L), R.2 (identities.rs 519L) extracted; `store.rs` still 3,787L (tests not yet split out) |
| `refactor(srv): split dispatch_service monolith (A4)` PR #1552 | `service.rs` split into per-service handlers | ✅ Done — but `service.rs` has regrown to 3,291L from new feature additions |
| `refactor(A12)` PR #1565 | Removed 65 dead `COMMAND_*` constants from `rpc_types.rs` | ✅ Done — still 2,454L, types not yet split by domain |
| `refactor(A3)` PR #1566 | Split `global.ts` god-module | ✅ Done |
| `refactor(A5)` PR #1564 | Extracted `BlockControllerCore` shared helpers | ✅ Done |
| `refactor(A11)` PR #1562 | Extracted `BlockRegistry` + `ModalLayer` dispatch | ✅ Done |
| `SPEC_AGENT_VIEW_SCSS_SPLIT_2026_04_24.md` | `agent-view.scss` (4,055 lines) decomposition | ⚠️ Spec exists, unclear if fully executed — needs verification |
| `SPEC_BROWSER_PANE_MODULARIZATION.md` | `browser_panes.rs` cycle-break | ⚠️ Spec exists, unclear if fully executed |
| `sysinfo-modularization-plan.md` | `sysinfo.tsx` (589 lines) split | ⚠️ Small file — may have been done or deferred |

### Key takeaway from prior work

- `reducer.rs` is **done** — remove from new plan.
- `websocket.rs` is **done** — remove from new plan.
- `store.rs` is **in progress** — R.1 and R.2 extracted; remaining work is R.3–R.4 (memory_bundles, content, skills, history already extracted per directory listing) and test extraction.
- `service.rs` has **regrown** since A4 — needs another pass.
- `agent_handlers.rs` and `app_api.rs` were never split — highest remaining priority.

---

## The Problem

Several files have grown to 1,500–4,500 lines by accumulating related-but-distinct concerns over time. This creates:
- Long compile times for hot-path Rust files (the entire file recompiles on any change)
- Hard-to-navigate code: reviewer context windows fill before reaching the relevant section
- Merge conflicts between agents working on separate logical domains in the same file

---

## File Inventory (>500 lines)

### Critical (>3,000 lines)

| File | Lines | Growth cause |
|------|-------|--------------|
| `agentmux-srv/src/server/agent_handlers.rs` | 4,504 | All RPC command handlers (V1–V7 feature waves) in one file |
| `agentmux-srv/src/server/app_api.rs` | 3,843 | All WebSocket app-API implementations in one file |
| `agentmux-srv/src/backend/storage/store.rs` | 3,787 | Core store + >100 integration tests inline |
| `agentmux-srv/src/reducer.rs` | 3,442 | Already partially modularized (7 subfiles); dispatcher itself is fine |
| `agentmux-srv/src/server/service.rs` | 3,291 | All HTTP service methods (window + pane + queries + dispatch) |
| `agentmux-cef/src/ui_tasks.rs` | 3,116 | All CEF UI tasks across every platform in one file |

### Large (1,500–3,000 lines)

| File | Lines | Notes |
|------|-------|-------|
| `agentmux-srv/src/backend/agent_session.rs` | 2,801 | Session read/write/archive + streaming |
| `agentmux-srv/src/backend/rpc_types.rs` | 2,454 | All RPC request/response types + command constants |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 2,394 | Shell PTY + subprocess + resize in one controller |
| `agentmux-cef/src/client/mod.rs` | 2,377 | CEF client callbacks (all platforms) |
| `agentmux-launcher/src/main.rs` | 2,264 | Launcher bootstrap + saga + IPC all in main |
| `frontend/types/gotypes.d.ts` | 2,338 | Generated — do not split manually |
| `frontend/app/store/rpc-api.ts` | 1,454 | 180+ RPC stubs, all domains in one class |
| `frontend/app/view/agent/agent-view.tsx` | 1,345 | Wrapper + hooks + render tree fused |
| `agentmux-srv/src/server/identity_handlers.rs` | 1,509 | OAuth session lifecycle; currently OK scope |

---

## Modularization Plan

### Phase 1 — Highest ROI, standalone splits (no cross-file refactor)

#### 1.1 `store.rs` → extract tests  
**Before:** `store.rs` (3,787 lines) = 670 lines of impl + ~3,100 lines of inline tests  
**After:**
```
agentmux-srv/src/backend/storage/
  store.rs          (~670 lines — impl only)
  tests/
    store_core.rs   (basic CRUD: insert/get/update/delete/count)
    store_agents.rs (agent definition lifecycle)
    store_identity.rs (identity accounts, bundles, bindings)
    store_memory.rs  (memory bundles, global brain)
    store_registry.rs (named-instance registry mirroring)
    store_dualwrite.rs (dual-write db ↔ global registry)
```
**Effort:** Low — pure file reorganization, no logic changes.  
**Rule:** `#[cfg(test)] mod tests` in `store.rs` becomes `#[cfg(test)] mod tests { mod core; mod agents; ... }` with `pub(crate) use` re-exports.

---

#### 1.2 `rpc_types.rs` → split by domain
**Before:** `rpc_types.rs` (2,454 lines) — all command constants + request/response types  
**After:**
```
agentmux-srv/src/backend/
  rpc_types/
    mod.rs          (re-exports everything; no new public API)
    commands.rs     (all COMMAND_* const strings)
    agent.rs        (agent.*  req/resp types)
    block.rs        (block.*, pane.*  req/resp)
    session.rs      (session.*  req/resp)
    identity.rs     (identity.*, account.*  req/resp)
    memory.rs       (memory.*, preset.*  req/resp)
    file.rs         (file.*, blockfile.*  req/resp)
    misc.rs         (config, event, ping, route, etc.)
```
**Effort:** Low — types don't reference each other across domains.  
**Note:** `rpc_types.rs` is imported by many files; `mod.rs` re-exports preserve the existing `use crate::backend::rpc_types::*` call sites unchanged.

---

#### 1.3 `ui_tasks.rs` → split by task category
**Before:** `ui_tasks.rs` (3,116 lines) — all CEF UI tasks across all platforms  
**After:**
```
agentmux-cef/src/
  ui_tasks/
    mod.rs              (re-exports all task types + post_* fns)
    window.rs           (Close, Minimize, Maximize, Focus, MemoryPressure)
    drag.rs             (StartWindowDrag — Linux + macOS variants)
    pane_geometry.rs    (SetPaneBoundsViews — ~1,600 lines, largest task group)
    platform_macos.rs   (macOS-specific swizzle storage, mac drag internals)
```
**Effort:** Medium — platform conditionals (#[cfg(target_os = ...)]) need careful placement.

---

### Phase 2 — Handler splits (require updating registration call sites)

#### 2.1 `agent_handlers.rs` → split by feature wave
**Before:** `agent_handlers.rs` (4,504 lines) — V1–V7 handlers all in one `register_*` function  
**After:**
```
agentmux-srv/src/server/
  agent_handlers/
    mod.rs              (single pub fn register_agent_handlers() — calls sub-registrars)
    core.rs             (V1–V5: listagents, create/update/deleteagent, fork, templatize)
    identity.rs         (V6: identity accounts, OAuth, named agents, instances)
    memory.rs           (V7: identity bundles, memory/brain system)
    session.rs          (session read/write/append/archive/export)
    subprocess.rs       (agent input, stop, spawn_subprocess, tool_decision)
    account_key.rs      (account.key.verify — Trust Center key flow)
```
**Registration change:** Each sub-module gets its own `pub fn register_*_handlers(engine, state)` called from `mod.rs`.  
**Effort:** Medium — handlers share `state` borrows but are otherwise independent.

---

#### 2.2 `app_api.rs` → split by RPC verb group
**Before:** `app_api.rs` (3,843 lines) — all WebSocket app-API impls  
**After:**
```
agentmux-srv/src/server/
  app_api/
    mod.rs              (register_app_api_handlers() + shared helpers)
    agent_open.rs       (agent.open — largest single handler ~400 lines)
    agent_io.rs         (agent.send, agent.stop, agent.status, agent.list, agent.output)
    pane.rs             (pane.open + pane sub-handlers)
    blockfile.rs        (blockfile.line_count, read_range, read_state, write_state)
    session.rs          (session.archive, restore, export, activity_summary)
    define.rs           (agent.define + agent config file writing)
    identity.rs         (identity.*, account.*, register_identity_account_validate)
    preset.rs           (preset.list, preset.get, preset.self.get)
    memory.rs           (memory.list, memory.read, memory.write)
    helpers.rs          (resolve_tab_id, find_agent_block, allocate_workdir, write_config_files)
```
**Note:** `storage/identities.rs` extraction (already done in Phase R.2) is the template for this pattern.  
**Effort:** High — many cross-references between handlers and helpers.

---

#### 2.3 `service.rs` → split by concern
**Before:** `service.rs` (3,291 lines)  
**After:**
```
agentmux-srv/src/server/
  service/
    mod.rs          (handle_service, run_service_call, dispatch_service, WebCallType)
    helpers.rs      (AgentContext, resolve_agent_context, workspace_id_for_tab, agent_layout)
    window.rs       (post_close_window, post_minimize, post_maximize, post_focus, etc.)
    pane.rs         (post_create_window, SetPaneBoundsViews, post_show_dev_tools, etc.)
    queries.rs      (get_window_position_blocking, get_window_rect_blocking, etc.)
    agent_zoom.rs   (schedule_agent_zoom_mirror — currently in websocket.rs import)
```
**Effort:** Medium.

---

### Phase 3 — Frontend splits

#### 3.1 `rpc-api.ts` → domain service classes
**Before:** `rpc-api.ts` (1,454 lines) — single `RpcApiType` class with 180+ methods  
**After:**
```
frontend/app/store/
  rpc-api/
    index.ts        (re-exports RpcApi as the composed object; no breaking change)
    agent.ts        (ListAgents, CreateAgent, UpdateAgent, DeleteAgent, ForkAgent, ...)
    block.ts        (CreateBlock, DeleteBlock, SetMeta, GetMeta, SetView, ...)
    file.ts         (FileRead, FileWrite, FileList, FileCopy, ...)
    session.ts      (SessionRead, SessionWrite, SessionArchive, ...)
    identity.ts     (Authenticate, ListIdentityAccounts, UpsertIdentityAccount, ...)
    memory.ts       (ListMemory, UpsertMemory, DeleteMemory, ReorderGlobalBrain)
    workspace.ts    (WorkspaceList, CreateTab, SetActiveTab, FocusWindow, ...)
    misc.ts         (EventPublish, EventSub, FetchSuggestions, PathCommand, ...)
```
**Compatibility:** `index.ts` exports `const RpcApi = { ...AgentApi, ...BlockApi, ... }` to preserve all existing call sites.  
**Effort:** Low-Medium — pure reorganization, no logic.

---

#### 3.2 `agent-view.tsx` → extract hooks
**Before:** `agent-view.tsx` (1,345 lines) — wrapper + 15+ hooks inlined in `AgentPresentationView`  
**After:**
```
frontend/app/view/agent/
  agent-view.tsx          (~300 lines — wrapper + presentation shell + render tree)
  hooks/
    useAgentCommands.ts   (already extracted — 585 lines, continue splitting)
    useAgentStream.ts     (already extracted — 938 lines, candidate for further split)
    useAgentScroll.ts     (scroll + virtualization hooks)
    useAgentSearch.ts     (search state hooks)
    useAgentKeyboard.ts   (keyboard binding hooks)
    useAgentSubagents.ts  (subagent event subscription hooks)
```
**Effort:** Medium — SolidJS signal dependencies need careful ordering.

---

## Sequencing

| Phase | Target files | Complexity | Expected PR size |
|-------|-------------|------------|-----------------|
| 1.1 | `store.rs` test extraction | Low | ~30 lines changed (just moves) |
| 1.2 | `rpc_types.rs` domain split | Low | ~50 lines changed (re-exports) |
| 3.1 | `rpc-api.ts` domain split | Low | ~50 lines changed (re-exports) |
| 1.3 | `ui_tasks.rs` category split | Medium | ~80 lines changed |
| 2.3 | `service.rs` concern split | Medium | ~100 lines changed |
| 2.1 | `agent_handlers.rs` wave split | Medium | ~150 lines changed |
| 2.2 | `app_api.rs` verb split | High | ~200 lines changed |
| 3.2 | `agent-view.tsx` hooks extraction | Medium | ~200 lines changed |

**Rule for all phases:** Module splits must not change behavior. Each PR should be pure reorganization — no logic changes, no renames of public symbols, and the diff should be close to zero for `git diff --stat` on non-`mod.rs` files.

---

## Non-goals

- `reducer.rs` — already modularized into 7 submodules. The dispatcher (`~225 lines`) is the right size.
- `gotypes.d.ts` — generated file; split at the generator level if needed.
- Files in the 500–1,000 line range — monitor but don't split preemptively.
- `identity_handlers.rs` (1,509 lines) — well-scoped OAuth session logic; split only if it grows further.
