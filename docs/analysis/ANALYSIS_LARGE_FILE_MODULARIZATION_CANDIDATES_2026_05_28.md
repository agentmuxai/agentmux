# Analysis: Large-file modularization candidates (2026-05-28)

**Author:** AgentA
**Status:** Survey doc + detailed sub-PR carve plans. R.1 of store.rs landed today; this updated revision adds full plans for the remaining 4 candidates.
**Related:** [SPEC_STORE_MODULARIZATION_2026_05_27.md](../specs/SPEC_STORE_MODULARIZATION_2026_05_27.md) — the playbook R.2–R.6 followed for `store.rs`.

---

## Why this exists

Today's session shipped 6 modularization PRs against `store.rs` (R.1–R.6), taking it from **6,226 → 3,306 lines (-47%)** and pulling out 7 cleanly-scoped sibling modules. The pattern is now well-rehearsed: identify a subsystem in a giant file, build a `mod.rs` re-export shim, add a sibling file with the subsystem's struct + `impl Store {}` block, leave a comment breadcrumb in the donor file. Each PR was ~150–650 LOC moved, ~30 minutes start to merged.

This doc maps the next set of fat files in the repo and ranks them by extraction value, with concrete sub-PR carve plans for each top candidate.

## Method

- `wc -l` over all `.rs` and `.ts/.tsx` files (excluding `node_modules`, `target`, `dist`, `.claude/worktrees`).
- For each top file, scanned the `pub fn` / `impl` / `// ==== section ====` boundaries to gauge how cleanly the file partitions.
- For each top candidate, listed the exact symbols + line spans + dependency surface, so the carve can be executed without re-reading the whole file.
- Cross-checked test-file ratios — a 4,000-line test file is a different problem than a 4,000-line source file.

## Current state (post-R.1)

| File | Pre-session | Post-session | Delta |
|---|---|---|---|
| `agentmux-srv/src/backend/storage/store.rs` | 6,226 | **3,306** | -47% ✅ |
| `agentmux-srv/src/backend/storage/agents.rs` | — | 1,556 | new (R.1) |
| `agentmux-srv/src/backend/storage/identities.rs` | — | ~480 | new (R.2) |
| `agentmux-srv/src/backend/storage/memory_bundles.rs` | — | ~290 | new (R.3) |
| `agentmux-srv/src/backend/storage/content.rs` | — | ~280 | new (R.4a) |
| `agentmux-srv/src/backend/storage/skills.rs` | — | ~220 | new (R.4b) |
| `agentmux-srv/src/backend/storage/history.rs` | — | ~250 | new (R.4c) |
| `agentmux-srv/src/backend/storage/dual_write.rs` | — | ~260 | new (R.5) |
| `agentmux-srv/src/backend/storage/registry_mirror.rs` | — | ~180 | new (R.6) |

**store.rs modularization plan is complete.** Next session can move on to the candidates below.

## Inventory — Rust (top 20 source files, excluding tests & worktrees)

| File | LOC | Verdict | Notes |
|---|---|---|---|
| `agentmux-launcher/src/reducer/tests.rs` | 4,113 | 🟢 **Skip** — test file. Splitting a test module by sub-system is fine but rarely worth the churn. Revisit only if test discovery becomes painful. |
| `agentmux-srv/src/server/agent_handlers.rs` | 3,456 | 🔴 **Top candidate.** 6 `register_*_handlers` functions stack into one file. Natural carves: agents + skills + history + identities + sessions + memories. **See detailed plan below.** |
| `agentmux-srv/src/backend/storage/store.rs` | 3,306 | 🟢 **Done** (R.1–R.6 shipped today). Further splits would chase phantom gains. |
| `agentmux-srv/src/reducer.rs` | 3,069 | 🟡 **Maybe** — pure state-machine code. Already partitioned internally by `Command` variant. Cleanest carve is by command family (window, workspace, tab, block, saga). Risk: tightly coupled tests; reducer tests rely on whole-state-machine assertions. Defer until tests are quieter. |
| `agentmux-srv/src/server/service.rs` | 2,581 | 🟡 **Maybe** — bootstrap glue. Some sections (WS upgrade, HTTP-asset, IPC server, broker bootstrap) could be separated. But this is "do once on startup" code with no hot path, so the maintenance cost is bounded. Lower priority. |
| `agentmux-srv/src/backend/agent_session.rs` | 2,396 | 🟡 **Maybe** — contains the `template_promote` migration (~700 LOC) and the per-block agent-session glue. Splitting `template_promote_migration.rs` out is a clean win; the rest is harder to factor. |
| `agentmux-srv/src/backend/rpc_types.rs` | 2,271 | 🟢 **Skip-ish** — type definitions only. The pain isn't *finding* a type — every editor handles 2k-line type files. Lower value than code-extraction targets. |
| `agentmux-common/src/ipc.rs` | 2,033 | 🟢 **Skip** — IPC enum + tag-dispatch table. Splitting an enum doesn't help readability. |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 1,836 | 🔴 **Candidate.** Controller subprocess management. Has clear functional zones. **See detailed plan below.** |
| `agentmux-launcher/src/saga/mod.rs` | 1,829 | 🟡 **Maybe** — `mod.rs` containing all the saga implementations is a smell; the dir already has `delete_block.rs`, `delete_workspace.rs`, etc. Each saga's definition + step impls could move out to a `<saga>.rs` peer. |
| `agentmux-cef/src/client/mod.rs` | 1,825 | 🔴 **Candidate.** The CEF client `mod.rs` carries all the CEF handler trait impls inline. **See detailed plan below.** |
| `agentmux-srv/src/server/app_api.rs` | 1,811 | 🟡 **Maybe** — same pattern as `agent_handlers.rs` but mixed RPC families. Lower priority. |
| `agentmux-srv/src/persist_subscriber.rs` | 1,588 | 🟢 **Skip** — sequential append-only event handling. Few subsystems to separate. |
| `agentmux-srv/src/backend/storage/agents.rs` | 1,556 | 🟢 **Just landed (R.1)** — leave alone. |
| `agentmux-srv/src/server/identity_handlers.rs` | 1,509 | 🟡 **Maybe** — already focused; identity-bundle + identity-account RPCs share helper context. |
| `agentmux-srv/src/server/websocket.rs` | 1,490 | 🟢 **Skip-ish** — connection + frame handling. Not much to gain. |
| `agentmux-cef/src/commands/window.rs` | 1,462 | 🔴 **Candidate — user-flagged.** **See detailed plan below.** |
| `agentmux-srv/src/identity/resolver.rs` | 1,434 | 🟡 **Maybe** — resolver is ~600 LOC, OAuth probe is ~400 LOC, tests are ~400 LOC. `oauth_probe.rs` carve is a clean win. |
| `agentmux-cef/src/state.rs` | 1,300 | 🟢 **Skip** — `AppState` struct + constructors. Already focused. |
| `agentmux-srv/src/backend/blockcontroller/subprocess.rs` | 1,254 | 🟡 **Maybe** — Win32 process spawning + pipe management. Could carve `windows_pipes.rs`, `process_spawn.rs`. Lower priority than `shell.rs`. |

## Inventory — TypeScript

| File | LOC | Verdict | Notes |
|---|---|---|---|
| `frontend/types/gotypes.d.ts` | 2,226 | 🟢 **Skip** — generated. Don't touch. |
| `frontend/app/store/agent-pane-state/reducer.test.ts` | 2,143 | 🟢 **Skip** — tests. |
| `frontend/app/store/rpc-api.ts` | 1,344 | 🟢 **Skip** — generated thin wrappers. |
| `frontend/app/store/browser-pane-state/reducer.test.ts` | 1,060 | 🟢 **Skip** — tests. |
| `frontend/app/view/agent/components/AgentLaunchModal.tsx` | 1,020 | 🔴 **Candidate** — big SolidJS component with multiple `Show when=`-gated subviews (identity tab, memory tab, name & cwd, continue-mode list). Carving each tab into its own file (`<TabName>Panel.tsx`) is the cleanest split. |
| `frontend/app/tab/tabbar.tsx` | 1,008 | 🟡 **Maybe** — drag-and-drop tab-bar. Could carve `tabbar-drag.ts` (hit-test + drag-state) out of the rendering logic. |
| `frontend/app/store/agent-document/reducer.test.ts` | 1,006 | 🟢 **Skip** — tests. |
| `frontend/app/store/global.ts` | 1,002 | 🟡 **Maybe** — Jotai atoms by topic. Could move each atom-family to its own file. Low cost, low pain — defer. |
| `frontend/app/view/agent/agent-view.tsx` | 958 | 🟡 **Maybe** — already partitioned by section components. Splitting harder than it looks. |

---

# Detailed sub-PR plans

Each plan below is concrete enough to execute without re-discovering the file structure. Symbol names, approximate line spans, and shared-state surfaces are listed.

## Plan 1: `agentmux-cef/src/commands/window.rs` (1,462 LOC → 6 sub-PRs)

User-flagged; cleanest partitions of the four candidates. Carve into `agentmux-cef/src/commands/window/`.

| New module | Symbols (line spans approx.) | LOC est. | Dependencies |
|---|---|---|---|
| `mod.rs` | `pub use lifecycle::*; pub use motion::*; pub use chrome::*; pub use transparency::*; pub use meta::*; pub use creation::*;` | ~15 | none |
| `lifecycle.rs` | `close_window` (55), `close_window_by_label` (78), `resolve_window_hwnd` (235), `find_main_window` (194), `find_own_top_level_window` (733), `find_all_own_windows` (703), `capture_hwnd_for_label` (1337), `FLOATING_PANE_CLASS_NAME` const (185) | ~300 | `state.window_hwnds`, Win32 `IsWindow` |
| `motion.rs` | `get_window_position` (342), `set_window_position` (405), `move_window_by` (368), `start_window_drag` (448), `resolve_window_at_cursor` (493), `update_floating_redock_hover` (595), `clear_floating_redock_hover` (640) | ~300 | `state.window_hwnds`, `state.floating_redock_hover` |
| `chrome.rs` | `minimize_window` (113), `maximize_window` (138) | ~50 | `state.window_hwnds` |
| `transparency.rs` | `set_window_transparency` (664), `set_window_opacity` (1386), `get_window_opacity` (1446), `apply_window_opacity` (767), `remove_window_opacity` (787) | ~150 | `state.window_hwnds`, Win32 `SetLayeredWindowAttributes` |
| `meta.rs` | `get_zoom_factor` (18), `set_zoom_factor` (26), `get_window_label` (798), `is_main_window` (805), `get_double_click_time` (820), `list_window_instances` (843), `list_windows` (883), `focus_window` (901), `get_instance_number` (912), `register_backend_window` (923), `toggle_devtools` (955), `inspect_element_at` (964) | ~280 | `state.window_hwnds`, `state.zoom_factor`, devtools APIs |
| `creation.rs` | `open_new_window` (1167), `open_subwindow` (1176), `open_window_with_kind` (1216), `resolve_frontend_base_url` (1039), `assets_missing_data_url` (1107), `dev_vite_port` (979), `resolve_host_runtime_dir` (1083), `get_offset_position` (1291), `get_secondary_window_size` (1314), `FrontendUrlError` enum | ~400 | `state.runtime_dir`, frontend bundle resolution |

**Recommended start:** `lifecycle.rs` first — contains close-button code path fixed in #1133, natural continuation. **Estimated total:** 6 sub-PRs × ~30 min = ~3 hours.

**Caller-update strategy:** `pub use` shim in `mod.rs` means callers (mostly `ipc.rs` dispatcher) don't have to change. Verify after each sub-PR with `cargo check -p agentmux-cef`.

---

## Plan 2: `agentmux-srv/src/server/agent_handlers.rs` (3,456 LOC → 4 sub-PRs)

This is the biggest non-storage file in the repo. It contains **6 `register_*_handlers` entry functions** that stack ~50+ individual `engine.register_handler!` blocks. The natural carve is one file per RPC family.

Current top-level structure (line spans):

| Entry function | Line | Approximate body span | Handler count |
|---|---|---|---|
| `register_agent_handlers` | 87 | 87–1,090 | ~22 handlers (agents, skills, history, identities, named_agents) |
| `register_v6_handlers` | 1,092 | 1,092–2,013 | ~15 handlers (agent_def_*, fork, hide, list_hidden) |
| `register_agent_session_handlers` | 2,015 | 2,015–2,145 | 4 handlers (session read/write/append/archive) |
| `register_v7_handlers` | 2,147 | 2,147–2,443 | ~10 handlers (identity_bundle_*, identity_binding_*, memory_*) |
| `read_session_preview` | 2,445 | 2,445–2,505 | helper |
| `collapse_preview` | 2,506 | 2,506+ | helper |

Carve into `agentmux-srv/src/server/agent_handlers/`:

| New module | Contents | LOC est. |
|---|---|---|
| `mod.rs` | `pub use core::register_agent_handlers; pub use v6::register_v6_handlers; pub use sessions::register_agent_session_handlers; pub use v7::register_v7_handlers;` + module-local helpers (`read_session_preview`, `collapse_preview`) | ~80 |
| `core.rs` | `register_agent_handlers` — agents/skills/history/identities/named_agents (the v5 surface) | ~1,000 |
| `v6.rs` | `register_v6_handlers` — agent_def_* + fork/hide/list_hidden | ~920 |
| `sessions.rs` | `register_agent_session_handlers` — read/write/append/archive + list_archives | ~130 |
| `v7.rs` | `register_v7_handlers` — identity bundles, identity bindings, memory CRUD | ~300 |

**Caller-update strategy:** `service.rs` (or wherever `register_*_handlers` get called) keeps the same imports because `mod.rs` re-exports them. Sub-PR order recommendation: `sessions.rs` first (smallest, lowest risk), then `v7.rs`, then `v6.rs`, then `core.rs`. Each sub-PR ~30 min. **Estimated total:** 4 sub-PRs × ~30 min = ~2 hours.

**Cross-cutting concern:** all handlers share `state.wstore`, `state.broker`, `state.identity_resolver`. The carve preserves these as constructor arguments, so the change is purely "where does the file live."

---

## Plan 3: `agentmux-cef/src/client/mod.rs` (1,825 LOC → 4 sub-PRs)

The CEF client `mod.rs` carries the `AgentMuxHandler` struct + all CEF handler trait impls inline. CEF handlers come in families (lifespan, load, request, display, render-process, browser-process) — each is a discrete trait impl block.

Carve into `agentmux-cef/src/client/`:

| New module | Contents | LOC est. |
|---|---|---|
| `mod.rs` | `AgentMuxHandler` struct, `new()`, registration of all sub-impls, `pub use` for cross-module helpers | ~250 |
| `lifespan.rs` | `impl LifeSpanHandler for AgentMuxHandler` — `on_after_created`, `on_before_close`, `do_close`, popup lifecycle, window-list bookkeeping | ~500 |
| `load.rs` | `impl LoadHandler for AgentMuxHandler` — `on_loading_state_change`, `on_load_start`, `on_load_end`, `on_load_error`, ready-state propagation | ~350 |
| `request.rs` | `impl RequestHandler` + `impl ResourceRequestHandler` — URL filtering, custom scheme handling, asset interception | ~400 |
| `display.rs` | `impl DisplayHandler` + misc — title/favicon/status/tooltip/console-message; status-bar updates | ~250 |
| `dispatch.rs` | The IPC message dispatch `on_process_message_received` (still the largest single fn) — if it's >400 LOC alone, this gets its own file. | ~250 |

**Higher risk than handlers** because the impl-block layout has to thread `self.state.inner.lock()` carefully; some lifespan callbacks coordinate with `do_close` via state flags. But the existing impl is already partitioned by trait, so the cuts are obvious.

**Sub-PR order:** `display.rs` first (smallest impl, lowest cross-impl coupling), then `load.rs`, then `request.rs`, then `lifespan.rs` (largest + most state coupling). **Estimated total:** 4 sub-PRs × ~45 min = ~3 hours.

**Caller-update strategy:** CEF trait impls don't need callers to change — registrations happen via the `Handler` trait method returns. All sub-modules add `impl Foo for AgentMuxHandler` blocks; Rust merges them at link time. No `pub use` shim needed for trait impls.

---

## Plan 4: `agentmux-srv/src/backend/blockcontroller/shell.rs` (1,836 LOC → 4 sub-PRs)

Controller subprocess management. Has clear functional zones for spawn, IO, lifecycle, and tests. Higher risk than store.rs because the state machine is intricate (Phase B reducer ports), but the file partitions cleanly.

Carve into `agentmux-srv/src/backend/blockcontroller/shell/`:

| New module | Contents | LOC est. |
|---|---|---|
| `mod.rs` | `ShellProc` struct + public API (`spawn`, `kill`, `input`, `resize`, `status`) — keeps the public surface | ~250 |
| `spawn.rs` | `spawn_shell`, Win32 ConPTY setup, Unix PTY setup, environment construction, shell-path resolution (calls into `detect_local_shell_path_*`) | ~500 |
| `io.rs` | PTY read loop, terminal data forwarding to broker, input-write fn, base64 encoding | ~400 |
| `lifecycle.rs` | Exit-code capture, SIGINT signaling, cleanup, retry policy | ~400 |
| `tests.rs` (already exists if module has tests) | unchanged | — |

**Risk:** the state machine across spawn → io → lifecycle is implicit; carving might force exposing internal handles via `pub(super)`. **Mitigation:** add one comment at the top of `mod.rs` documenting the lifecycle contract before extracting.

**Sub-PR order:** `spawn.rs` first (most self-contained), then `io.rs`, then `lifecycle.rs`. Defer the test module split. **Estimated total:** 4 sub-PRs × ~45 min = ~3 hours.

---

## Recommended next moves (priority order)

1. **`window.rs` carve (Plan 1)** — user-flagged; clean partitions; 6 sub-PRs but the first one is the close-button code we just fixed today. Natural momentum.
2. **`agent_handlers.rs` carve (Plan 2)** — biggest win-per-PR; pure RPC handler split. The shared `WshRpcEngine` argument is the only cross-cutting concern, and it's already an argument so no refactor needed.
3. **`client/mod.rs` carve (Plan 3)** — splits the CEF handler trait impls. Higher risk than handlers because of state coupling, but the trait boundaries make the cuts obvious.
4. **`shell.rs` carve (Plan 4)** — lower priority because the state machine is intricate and the file is "only" 1,836 LOC. Defer until 1, 2, 3 are done and there's appetite for the risk.

After those four land, every Rust file in the repo is under 1,800 LOC. **That's the natural stopping point** — further splits start to chase phantom gains.

## What we are NOT going to extract

- **Test files** (they grow with feature count; if a test file is slow, the answer is test-runner sharding, not module splits).
- **Generated files** (`gotypes.d.ts`, `rpc-api.ts`).
- **Tiny config/constants files** (under 200 LOC).
- **`enum` + `tag dispatch` files** (`ipc.rs`, `rpc_types.rs`) — splitting an enum spreads exhaustiveness checks across files for no readability gain.
- **`store.rs` further splits** — R.1–R.6 cleared the modularization-worth boundaries; what remains is the irreducible Store core.

## Pattern: the R-style modularization playbook

For any flat file with `>1500 LOC` and clean functional partitions:

1. **Identify** subsystems via `grep -nE "^pub fn|^impl"` and reading section comments.
2. **Create** a sibling directory `<name>/` with a `mod.rs` that re-exports from sub-modules.
3. **Move** each subsystem to its own file, preserving identical signatures.
4. **Add** `pub use submod::*;` (for flat functions) or sibling `impl Type {}` blocks (for methods).
5. **Leave** a one-line breadcrumb comment in the donor file: `// Moved to submod.rs (R.N)`.
6. **Verify** with `cargo check -p <crate>` after each sub-PR; the compiler is the safety net.
7. **One sub-PR per subsystem.** Each is reviewable as a pure-move diff; mixing logic changes defeats the point.

This pattern has shipped 7 times today (R.0–R.6 for store.rs) and is the reference for the 4 plans above.

## Closing note

Pre-session: 6 Rust files >2,000 LOC in the source tree. Post-R.6: still 6 (store.rs went from 6,226 to 3,306; agents.rs is new at 1,556). After Plans 1–4 ship: **3** Rust files >2,000 LOC (reducer.rs, service.rs, agent_session.rs), all 🟡 *Maybe* — and every one of them has its own bespoke risk profile that doesn't fit the R-style playbook.

The work to "get every source file under 1,800 LOC" is bounded at ~12 hours of focused PRs.
