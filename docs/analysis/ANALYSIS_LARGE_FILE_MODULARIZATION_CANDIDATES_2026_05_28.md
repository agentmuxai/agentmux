# Analysis: Large-file modularization candidates (2026-05-28)

**Author:** AgentA
**Status:** Survey doc — not a commitment. Use as an inventory when picking the next R-style refactor.
**Related:** [SPEC_STORE_MODULARIZATION_2026_05_27.md](../specs/SPEC_STORE_MODULARIZATION_2026_05_27.md) — the playbook that R.2–R.6 followed for `store.rs`.

---

## Why this exists

Today's session shipped 5 modularization PRs against `store.rs`, taking it from 6,226 → 4,836 lines and pulling out 6 cleanly-scoped sibling modules. The pattern is now well-rehearsed: identify a subsystem in a giant file, build a `mod.rs` re-export shim, add a sibling file with the subsystem's struct + `impl Store {}` block, leave a comment breadcrumb in the donor file. Each PR was ~150–650 LOC moved, ~30 minutes start to merged.

This doc maps the next set of fat files in the repo and ranks them by extraction value. It is **not** a commitment — it's the menu the next person resuming this work can pick from.

## Method

- `wc -l` over all `.rs` and `.ts/.tsx` files (excluding `node_modules`, `target`, `dist`).
- For each top file, scanned the `pub fn` / `impl` / `// ==== section ====` boundaries to gauge how cleanly the file partitions.
- Cross-checked test-file ratios — a 4,000-line test file is usually a different problem than a 4,000-line source file.

## Inventory — Rust

| File | LOC | Verdict | Notes |
|---|---|---|---|
| `agentmux-srv/src/backend/storage/store.rs` | 4,836 | 🟡 **R.1 pending** — the agents subsystem (~1,500 LOC: `agent_def_*`, `instance_*`, `AgentDefinition`, `AgentInstance`, `InstanceStatus`) is the last piece of the modularization plan in `SPEC_STORE_MODULARIZATION_2026_05_27.md`. Carved out: `R.0` rename, `R.2` identities, `R.3` memory_bundles, `R.4` content/skills/history, `R.5` dual_write (deletes in Phase 3c), `R.6` registry_mirror. Once R.1 lands the file drops below 3,500. |
| `agentmux-launcher/src/reducer/tests.rs` | 4,113 | 🟢 **Skip** — test file. Splitting a test module by sub-system is fine but rarely worth the churn. Revisit only if test discovery becomes painful. |
| `agentmux-srv/src/server/agent_handlers.rs` | 3,456 | 🔴 **Candidate** — every `register_handler!` block is an independent unit. Natural carves: `agent_def_handlers.rs` (createagent / updateagent / deleteagent / listagents / agentdefhide), `agent_instance_handlers.rs` (createagentinstance / updateagentinstance / deleteagentinstance / listagentinstances / getagentinstance), `named_agent_handlers.rs` (listnamedagents / continuenamedagent / hidenamedagent), `template_promote_handlers.rs`. Estimated 4 sub-PRs, ~600–900 LOC each. **Cross-cutting concern:** all of these share `state.wstore` + `state.broker` and a common `WshRpcEngine` registration ritual; carving needs a `register_all` entry that each sub-module contributes to. |
| `agentmux-srv/src/reducer.rs` | 3,069 | 🟡 **Maybe** — pure state-machine code. Already partitioned internally by `Command` variant. Cleanest carve is by command family (window, workspace, tab, block, saga). Risk: tightly coupled tests; reducer tests rely on whole-state-machine assertions. Defer until tests are quieter. |
| `agentmux-srv/src/server/service.rs` | 2,581 | 🟡 **Maybe** — bootstrap glue. Some sections (the WS upgrade path, the HTTP-asset path, the IPC server, the broker bootstrap) could be separated. But this is "do once on startup" code with no hot path, so the maintenance cost is bounded. Lower priority. |
| `agentmux-srv/src/backend/agent_session.rs` | 2,396 | 🟡 **Maybe** — contains the `template_promote` migration (~700 LOC) and the per-block agent-session glue. Splitting `template_promote_migration.rs` out is a clean win; the rest is harder to factor. |
| `agentmux-srv/src/backend/rpc_types.rs` | 2,271 | 🟡 **Skip-ish** — type definitions only. The pain isn't *finding* a type — every editor handles 2k-line type files. Lower value than code-extraction targets. Could split by RPC family if review velocity becomes a bottleneck. |
| `agentmux-common/src/ipc.rs` | 2,033 | 🟢 **Skip** — IPC enum + tag-dispatch table. Splitting an enum doesn't help readability. |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 1,836 | 🔴 **Candidate** — controller subprocess management. Splitting could yield `shell_spawn.rs`, `shell_io.rs`, `shell_lifecycle.rs`, `shell_tests.rs`. Bigger risk than store.rs because the state machine is intricate (Phase B reducer ports), but the file has clear functional zones. |
| `agentmux-launcher/src/saga/mod.rs` | 1,829 | 🟡 **Maybe** — `mod.rs` containing all the saga implementations is a smell; the dir already has `delete_block.rs`, `delete_workspace.rs`, etc. Each saga's "definition + step impls" pair could move out to a `<saga>.rs` peer. |
| `agentmux-srv/src/server/app_api.rs` | 1,811 | 🟡 **Maybe** — same pattern as `agent_handlers.rs` but mixed RPC families. Lower priority than the agent handlers because it's already mid-density. |
| `agentmux-cef/src/client/mod.rs` | 1,803 | 🔴 **Candidate** — the CEF client `mod.rs` carries the `AgentMuxHandler` impl block with all the lifecycle (lifespan, load, request, display, etc.) handlers inline. Natural carve: one file per CEF handler interface (`lifespan.rs`, `load.rs`, `request.rs`, `display.rs`). Same `impl AgentMuxHandler` split pattern as store.rs's `impl Store` split. ~4 sub-PRs, ~400 LOC each. |
| `agentmux-srv/src/persist_subscriber.rs` | 1,588 | 🟢 **Skip** — sequential append-only event handling. Few subsystems to separate. |
| `agentmux-srv/src/server/identity_handlers.rs` | 1,509 | 🟡 **Maybe** — already focused; carving wouldn't yield clean boundaries because identity-bundle + identity-account RPCs share a lot of helper context. |
| `agentmux-srv/src/server/websocket.rs` | 1,490 | 🟢 **Skip-ish** — connection + frame handling. Not much to gain from splitting. |
| `agentmux-cef/src/commands/window.rs` | 1,445 | 🔴 **Candidate — user-flagged** | See detailed plan below. |
| `agentmux-srv/src/identity/resolver.rs` | 1,434 | 🟡 **Maybe** — the actual resolver is ~600 LOC; the rest is OAuth probe code (~400 LOC) and the test module (~400 LOC). Carving `oauth_probe.rs` out would help, since the probe is independent of the resolver dispatch. |
| `agentmux-cef/src/state.rs` | 1,300 | 🟢 **Skip** — AppState struct + a handful of constructors. Already focused. |
| `agentmux-srv/src/backend/blockcontroller/subprocess.rs` | 1,254 | 🟡 **Maybe** — Win32 process spawning + pipe management. Could carve `windows_pipes.rs`, `process_spawn.rs`. Lower priority than `shell.rs`. |
| `agentmux-srv/src/main.rs` | 1,134 | 🟢 **Skip** — bootstrap. Inevitable that it's long; refactoring wouldn't help readability. |
| `agentmux-common/src/data_paths.rs` | 1,091 | 🟢 **Skip** — path-resolution constants + helpers. Already organized. |
| `agentmux-launcher/src/main.rs` | 1,077 | 🟢 **Skip** — launcher main loop. Similar reasoning to srv main. |
| `agentmux-srv/src/backend/storage/migrations.rs` | 1,036 | 🟡 **Maybe** — each `migrate_to_vN` could be its own file. Easy carve but limited churn benefit. Defer. |
| `agentmux-cef/src/launcher_ipc.rs` | 1,022 | 🟢 **Skip** — small surface (IPC client). |

## Inventory — TypeScript

| File | LOC | Verdict | Notes |
|---|---|---|---|
| `frontend/types/gotypes.d.ts` | 2,226 | 🟢 **Skip** — generated. Don't touch. |
| `frontend/app/store/agent-pane-state/reducer.test.ts` | 2,143 | 🟢 **Skip** — tests. |
| `frontend/app/store/rpc-api.ts` | 1,344 | 🟢 **Skip** — generated thin wrappers. |
| `frontend/app/store/browser-pane-state/reducer.test.ts` | 1,060 | 🟢 **Skip** — tests. |
| `frontend/app/view/agent/components/AgentLaunchModal.tsx` | 1,020 | 🔴 **Candidate** — big React component with multiple `Show when=`-gated subviews (identity tab, memory tab, name & cwd, continue-mode list). Carving each tab into its own file (`<TabName>Panel.tsx`) is the cleanest split. |
| `frontend/app/tab/tabbar.tsx` | 1,008 | 🟡 **Maybe** — drag-and-drop tab-bar. Could carve `tabbar-drag.ts` (hit-test + drag-state) out of the rendering logic. |
| `frontend/app/store/agent-document/reducer.test.ts` | 1,006 | 🟢 **Skip** — tests. |
| `frontend/app/store/global.ts` | 1,002 | 🟡 **Maybe** — Jotai atoms by topic. Could move each atom-family to its own file. Low cost, low pain — defer. |
| `frontend/app/view/agent/agent-view.tsx` | 958 | 🟡 **Maybe** — already partitioned by section components. Splitting harder than it looks. |

## Detailed plan: `agentmux-cef/src/commands/window.rs`

This file is user-flagged and has cleanly-grouped sections. Proposed carve into a `agentmux-cef/src/commands/window/` directory:

| New module | Contents | LOC est. |
|---|---|---|
| `mod.rs` | re-exports for the existing flat `commands::window::*` import sites | ~15 |
| `lifecycle.rs` | `close_window`, `close_window_by_label`, `resolve_window_hwnd`, `capture_hwnd_for_label`, `find_own_top_level_window`, `find_main_window`, the `EnumWindows` helpers | ~350 |
| `motion.rs` | `get_window_position`, `set_window_position`, `move_window_by`, `start_window_drag`, `resolve_window_at_cursor`, `update_floating_redock_hover`, `clear_floating_redock_hover` | ~300 |
| `chrome.rs` | `minimize_window`, `maximize_window` | ~200 |
| `transparency.rs` | `set_window_transparency`, `set_window_opacity`, `get_window_opacity` | ~150 |
| `meta.rs` | `get_window_label`, `is_main_window`, `list_window_instances`, `list_windows`, `focus_window`, `get_instance_number`, `register_backend_window`, `get_zoom_factor`, `set_zoom_factor`, `get_double_click_time`, `toggle_devtools`, `inspect_element_at` | ~250 |
| `creation.rs` | `open_new_window`, `open_subwindow`, the `FrontendUrlError` family, `resolve_frontend_base_url`, `assets_missing_data_url` | ~250 |

Estimated 6 sub-PRs, each ~30 minutes. Same playbook as `store.rs`: each sub-PR uses a `pub use` shim in `mod.rs` so callers don't have to update their `use` paths.

## Recommended next moves (priority order)

1. **R.1 store.rs agents extraction** — already-spec'd, blocked only by review-time. Finishing this lands the modularization plan that's two-thirds done.
2. **`window.rs` carve** (~6 sub-PRs) — user-flagged; clean partitions; no semantic risk since these are flat command handlers with shared state pattern.
3. **`agent_handlers.rs` carve** (~4 sub-PRs) — biggest win-per-PR; pure RPC handler split. The cross-cutting `register_all` rejigger is a one-time cost.
4. **`client/mod.rs` (CEF) carve** (~4 sub-PRs) — splits the CEF handler interfaces. Higher risk than handlers because the impl-block layout has to thread `inner.lock()` carefully — but the existing impl is already partitioned by trait, so the cuts are obvious.

After those four, every Rust file in the repo is under 1,800 LOC. That's the natural stopping point — further splits start to chase phantom gains.

## What we are NOT going to extract

- Test files (they grow with feature count; if a test file is slow, the answer is test-runner sharding, not module splits).
- Generated files (`gotypes.d.ts`, `rpc-api.ts`).
- Tiny config/constants files (under 200 LOC).
- The `enum` + `tag dispatch` files (`ipc.rs`, `rpc_types.rs`) — splitting an enum spreads exhaustiveness checks across files for no readability gain.

## Closing note

This survey took ~10 minutes to assemble. The pattern from R.2–R.6 (sibling module + `impl Store {}` block + comment breadcrumb) generalizes — anything that compiles as a flat collection of `pub fn`s sharing a small state argument is a candidate. The bar is "does this file partition cleanly into > 3 functional zones each addressable to a different sub-team?" If yes, modularize. If no, leave alone.
