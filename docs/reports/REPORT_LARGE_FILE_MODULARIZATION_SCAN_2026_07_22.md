# Large-File Modularization Scan

**Date:** 2026-07-22
**Baseline:** `agentmux` main @ latest (post `chore(cleanup)` PRs #2257/#2267)
**Method:** Line-count scan across `agentmux-srv`, `agentmux-cef`, `agentmux-launcher`, `agentmux-common`, and `frontend`, thresholded at ~600+ lines (Rust) / ~600+ lines (TS/TSX, excluding tests and generated `.d.ts`), followed by structural read-through of every candidate file (impl-block/function/component inventory, doc-comment review, cross-reference of what's genuinely shared vs. incidentally colocated) to separate **files that are one cohesive concern that's just long** from **files that bundle multiple weakly-coupled responsibilities**. 84 files were read and judged individually — no verdict was assigned from line count alone.

**Scope note:** This is a *structural* audit — proposed splits, not a redesign. Nothing here changes behavior; every proposal is a mechanical extraction (move code to a new file, update imports) unless explicitly flagged otherwise. No code was changed as part of this pass.

---

## 0. Executive summary

Of 84 files scanned:

| Verdict | Rust (`srv`) | Rust (`cef`/`launcher`) | Frontend (agent-pane) | Frontend (general) | Total |
|---|---|---|---|---|---|
| **SPLIT** | 15 | 15 | 11 | 21 | **62** |
| **SPLIT (partial/optional)** | — | — | 2 | — | **2** |
| **KEEP-AS-IS** | 6 | 5 | 2 | 7 | **20** |

The dominant pattern, repeated dozens of times across both languages, is **handler/dispatch files that grew by accretion**: a `match`/`switch` over N command or route variants, where each arm is a fully independent, weakly-coupled unit that only shares a common dispatch table. These are the highest-value, lowest-risk splits — no shared private state to untangle, just "move N independent functions to M files grouped by domain." Rust's `server/service/workspace.rs` and `server/service/window.rs` (single giant `match` over RPC methods) and the frontend's `keymodel.ts`/`action-widgets.tsx`/`markdown.tsx` all fall in this bucket.

A secondary, more valuable pattern: several files bundle a **stateful/reactive core** with **incidental pure utilities** that happened to be written in the same file — `agent-model.ts`'s config-file builders, `providers/index.ts`'s static data table vs. its dynamic model-overlay logic, `state.rs`'s ~15 standalone type definitions living 1000+ lines above the `AppState` struct that actually uses them. These splits are also low-risk (the pure pieces have zero coupling to the stateful core) and meaningfully improve navigability.

A third pattern, and the one requiring the most care: files where the size is **inherent to one legitimately complex, order-sensitive algorithm** (`AgentDocumentVirtualList.tsx`'s virtualization core, `useAgentStream.ts`'s single-RAF-batch flush design, `identity/resolver.rs`'s security-gate function, `subprocess.rs`'s two independent process-lifecycle state machines). These are still SPLIT candidates in most cases, but several carry **explicit regression-history warnings in their own comments** — a prior production incident that the current single-file/single-batch shape was specifically built to prevent. Any implementation of these splits must preserve the documented invariant, not just move code that still compiles. These are called out individually below.

**Two cross-cutting bonus findings** surfaced during the read-through, beyond pure line-count modularization:
- **`frontend/layout/lib/TileLayout.{win32,linux,darwin}.tsx`** each independently define byte-identical copies of `NodeBackdrops`, `MagnifiedPaneOverlay`, `DisplayNodesWrapper`, and `Placeholder` — ~250 duplicated lines × 3 platforms = **~750 lines of exact duplication** that could collapse into one shared `tilelayout-shared.tsx`, without touching the genuinely platform-specific drag/resize logic in each file.
- **`frontend/app/view/agent/components/AgentPicker.tsx`** contains two functions, `openLaunchModal` and `autoContinue`, that are defined and exported but — per grep — never called or imported anywhere in the current codebase. Likely dead code left over from a removed picker flow; worth a follow-up deletion pass rather than folding into any split.

---

## 1. Rust — `agentmux-srv` (backend)

21 files scanned, all read in full (or near-full) by direct structural analysis.

### 1.1 Highest-value candidates (detailed)

#### `reducer.rs` (4044 lines) — **SPLIT**
93% of this file (lines 292–4044, ~3750 lines) is a single flat `#[cfg(test)] mod tests` covering every command domain (lifecycle, workspace, tab, block, window, move_tab/move_block, and a ~1660-line layout+proptest section) — even though the *production* code is already cleanly split into `mod block; mod layout; mod lifecycle; mod snapshot; mod tab; mod window; mod workspace;`, each with **zero tests of its own**. The fix: move each domain's tests into its own submodule (mirroring the production split that already exists), leaving `reducer.rs` at ~290 lines (mod declarations + `Ctx` + the `update()` dispatch match). Shared test fixtures (`ctx`, `create_workspace`, `assert_invariants`, proptest strategies) need a small `reducer/test_support.rs` rather than duplication. This is the single biggest line-count win in the whole scan.

#### `backend/subagent_watcher.rs` (3717 lines) — **SPLIT**
One `impl SubagentWatcher` block (~1584 lines) bundles lifecycle (`spawn`/`watch_agent`/`unwatch_agent`/`prune_block`), a query/naming API (`list_active`/`get_history`/`set_display_name`), directory scanning/reconciliation, and jsonl/journal stream processing (`process_jsonl_change` alone is 328 lines) — plus ~465 lines of free-standing path/parsing utilities and ~1295 lines of tests. Proposed: `subagent_watcher/{mod,query,scan,jsonl,parse,types}.rs`, splitting along those four already-distinguishable concerns as child modules (preserving private-field access via Rust's module-tree visibility, not `pub(crate)` spam). `process_jsonl_change` should move whole, not be decomposed internally as part of this pass.

#### `identity/resolver.rs` (2193 lines) — **SPLIT, with a security caveat**
Bundles OAuth token-file probing (~176 lines), error types (~82 lines), a provider-classification table (~113 lines), secret resolution (~73 lines), and the actual credential-injection gate `inject_identity_env_with_broker` (~314 lines) — plus 1254 lines of tests (57% of the file). The peripheral pieces split cleanly and safely. **The gate function itself must not be split further or separated from its tests**: the file's own doc comment is an explicit incident warning — *"this module is where SPEC_PROVIDER_ISOLATION's INV-A is enforced — or... silently stopped being enforced,"* referencing `retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`. Move it as one unit to `identity/resolver/inject.rs`; extract everything else around it.

#### `main.rs` (1502 lines) — **SPLIT**
`async fn main()` alone is ~1165 lines (78% of the file) — one flat, numbered bootstrap sequence (crash-monitor → watchdog → logging → CLI/config → DB/migration → event bus + background tasks → listener binds → router/AppState build → shutdown handlers). Proposed: extract each numbered phase into a `bootstrap::` function (`install_process_watchers`, `load_config`, `open_stores_and_migrate`, `spawn_background_subsystems`, `bind_listeners_and_build_router`, `install_shutdown_handlers`), leaving `main()` as a ~100–150-line orchestrator. This is a bigger mechanical lift than most other candidates (heavy sequential `Arc<...>` threading between phases) but touches no shared/reused logic.

#### `server/service/workspace.rs` (1358 lines) and `server/service/window.rs` (1327 lines) — **SPLIT (cleanest candidates in the whole Rust scan)**
Each file is **one function** — `handle_workspace_service`/`handle_window_service` — containing a single `match call.method.as_str() { ... }` with 16–18 arms, each a fully self-contained async RPC handler (`state: &AppState`, `call: &WebCallType` passed explicitly, no shared private state, arms range ~15–320 lines). `window.rs`'s tests are already grouped into 5 feature-scoped sub-modules that map 1:1 onto the proposed split. Proposed: group arms by lifecycle phase into 4 files each (e.g. workspace → `workspace_lifecycle.rs`/`tab_lifecycle.rs`/`tab_move.rs`/`tear_off.rs`; window → `window_query.rs`/`window_create.rs`/`window_close.rs`/`window_mutate.rs`), with `mod.rs` shrinking to a ~40-line dispatcher. Zero complications — purest "extract match arm to function" refactor found in the scan.

#### `backend/storage/agents.rs` (1852 lines) — **SPLIT**
Two structurally parallel, independently-evolving CRUD surfaces for two different SQLite tables — agent **definitions** (~650 lines: `agent_def_get/list/insert/update/delete/...`) and agent **instances** (~885 lines: `instance_list/get/create/update/...`, including the 190-line `instance_list_named` with its recursive-CTE continuation-chain dedup) — connected only by a cascade-delete relationship and a shared `self.conn`. The file's own internal comment already marks this boundary. Matches the codebase's existing convention of one `impl Store {}` block per concern in its own file (`content.rs`, `skills.rs`, `history.rs` already do this). Proposed: `storage/agent_definitions.rs` (~750 lines) + `storage/agent_instances.rs` (~1000 lines).

#### `server/identity_handlers.rs` (1710 lines) — **SPLIT**
Despite its name, this file is scoped to pre-launch OAuth-flow RPC handlers only, but bundles four technical layers: RPC dispatch/wire types (~350 lines), a subprocess spawn/drain engine with near-duplicate pipes-vs-PTY variants (~670 lines — nearly half the file), filesystem/directory provisioning (~245 lines), and DB persistence (~150 lines) — a proportionate 289-line test module rounds it out. Proposed: `identity_handlers.rs` (slimmed, ~350) + `identity_auth_spawn.rs` (~700) + `identity_auth_dirs.rs` (~245) + `identity_auth_persist.rs` (~150). Dependency graph is one-directional (dispatch → dirs, dispatch → spawn → persist), no cycles; several currently-private helpers need `pub(crate)` visibility. **Bonus finding**: `compute_and_ensure_bundle_dir` is explicitly documented as vestigial (bundle mode retired) — flag as a deletion candidate for a future cleanup pass, not just a move candidate.

#### `backend/blockcontroller/subprocess.rs` (1770 lines) — **SPLIT**
Bundles controller bookkeeping (small, cohesive), a session-id continuity state machine (small, already isolated via a deliberate `&Mutex<Inner>`-not-`&self` signature so async tasks can call it without `Arc<Self>`), a **host-subprocess turn** (`spawn_turn`, 565 lines, `std::process`-based), and a **container-exec turn** (`spawn_container_turn` + `publish_line`, ~430 lines, `bollard`/Docker-socket-based). The two spawn paths implement the same conceptual protocol against different substrates and never call each other. Proposed: convert to `subprocess/{mod,session,host_spawn,container_spawn,tests}.rs`. Worth factoring the 4×-duplicated `BlockControllerRuntimeStatus` snapshot-construction into one shared helper during the split rather than copying the duplication forward.

### 1.2 Remaining SPLIT candidates (compact)

| File | Lines | Summary |
|---|---|---|
| `persist_subscriber.rs` | 2426 | ~25 near-identical `apply_*` functions (one per `Event` variant) already naturally group by domain (workspace/tab/block/window/layout), mirroring `reducer.rs`'s own submodule split. ~1173 lines are tests. Mechanical, lower-priority (file reads fine linearly today). |
| `server/mod.rs` | 1265 | `AppState`/`build_router`/`health_handler`/`auth_middleware` core (~350 lines) plus ~30 independent HTTP handlers with no shared state, groupable into `diag_handlers.rs`/`shell_proxy_handlers.rs`/`agent_query_handlers.rs`/`naming_handlers.rs`/`list_handlers.rs`. |
| `server/websocket.rs` | 1181 | Connection lifecycle (~500 lines, one cohesive read/dispatch/coalesce loop) vs. `register_handlers`' 18 independent, fully self-contained `engine.register_handler(...)` closures (594 lines, half the file) — group the latter by domain. |
| `registry/migrate.rs` | 1508 | Four phases: shared discovery (~120 lines), full one-time migration pipeline (~225), lighter backfill pipeline (~136), row-level conversion (~145). ~841 lines tests. `registry/migrate/{discovery,migrate,backfill,row_convert}.rs`. |
| `backend/blockcontroller/persistent.rs` | 1401 | `spawn_process` alone is 508 lines (spawn + stdout/stderr readers + session capture + health monitoring) vs. construction/messaging/trait-glue (~630 lines). Move `spawn_process` + its tightly-coupled `push_stdin`/`handle_control_frame` to `persistent/spawn.rs` as one unit — it's one continuous, order-sensitive state machine, don't decompose further inside it. |
| `backend/obj.rs` | 985 | Meta-serialization helpers / block-config types / core `StoreObj` entity hierarchy — three genuinely distinguishable, uncoupled groups of plain data types. Low-risk, low-urgency (~438 lines are tests). |

### 1.3 KEEP-AS-IS (with reasoning)

| File | Lines | Why |
|---|---|---|
| `server/app_api/mod.rs` | 1291 | Already decomposed into 10 submodules; what remains (`register_app_api_handlers` + a handful of `*_impl` entry points, ~366 lines of feature-scoped tests) is the correct residual, not bundled logic. |
| `backend/layout/mod.rs` | 1147 | Pure tree-algorithm library for one recursive `LayoutNode` structure — every function operates on the same data type and is used together by the reducer's `layout.rs`. Tests already externalized. Textbook "one cohesive concern, long because it's a complete algorithm library." |
| `server/agent_handlers/mod.rs` | 1113 | Production code already split into 9 submodules; remaining ~980 lines are one exhaustive (legitimately so) test module for two small preview-extraction helpers. |
| `backend/wps.rs` | 1058 | One pub/sub `Broker` + its tightly-coupled private scope-matching helpers. ~510 lines are tests. Splitting the helpers from the one impl block that uses them buys nothing. |
| `backend/storage/migrations.rs` | 1320 | Dominated by `run_object_schema`'s 415-line sequential DDL list — the "big const/DDL table" pattern that's legitimately long by nature, not bundled unrelated responsibilities. (An *optional* split into per-store schema files is defensible but not required.) |
| `reducer/layout.rs` | 874 | One cohesive concern (layout-command reducer arms, 1:1 with `Command::Layout*` variants). Its apparent size in the original scan is attributable to `reducer.rs`'s test module living in the wrong file (§1.1) — this file itself has zero tests and is appropriately scoped. |

---

## 2. Rust — `agentmux-cef` (host) + `agentmux-launcher`

20 files scanned; cross-validated by two independent structural passes (verdicts agreed in all 20 cases).

### 2.1 Highest-value candidates (detailed)

#### `commands/window_pool.rs` (2256 lines) — **SPLIT**
Two structurally parallel, independently-lifecycled subsystems already self-labeled in the source: the **top-level window pool** (`?pool=1` full windows, ~1690 lines) and the **pane pool** (`floating-pool-*` frameless panes, ~600 lines, explicitly commented `// ── Pane pool ─────`). Each has its own HWND cache, target-size const, and spawn/register/promote/orphan-cleanup function family. Proposed: extract `commands/pane_pool.rs`. 5 external call sites (`floating_pane.rs`, `memory_heartbeat.rs`, `client/lifecycle.rs`, `commands/drag.rs`, `commands/floating_pane.rs`) reach pane-pool functions via `window_pool::…` — either update them or `pub use pane_pool::*;` from `window_pool.rs`.

#### `state.rs` (2044 lines) — **SPLIT**
~1370 lines of ~15 largely-independent type definitions (drag, window-meta, browser-pane, pool, quit, top-level-creation-saga, UI-thread-gate types) sit *above* the single `AppState` struct that actually uses them as field types. Two of these clusters (`PendingReprojectClosures`, `UiThreadGate`) already have their own embedded `#[cfg(test)]` modules — a strong signal they're already logically standalone, just physically parked in the god file. Proposed: ~10 `state/*.rs` submodules for the auxiliary types, each `pub use`-re-exported so `crate::state::X` paths elsewhere don't change; `state.rs` itself keeps `AppState`'s struct/`Default`/accessor-`impl` core (~900–1220 lines, still substantial but genuinely cohesive). No circular dependency (auxiliary types never reference `AppState`).

#### `commands/platform.rs` (1930 lines) — **SPLIT**
Bundles small system-info IPC getters (`get_platform`/`get_user_name`/`get_data_dir`/etc., ~370–620 lines depending on where the boundary is drawn) with an entirely separate **CLI provider-login subsystem** (~900–1300 lines: `CliLoginStdin`, `run_cli_login`/`run_cli_login_pty`, PTY spawning, secret redaction, its own dedicated test modules) — verified via grep that the login cluster is referenced only from within this file, `state.rs` (one field type), and `ipc.rs` (dispatch). Proposed: extract wholesale into `commands/cli_login.rs`; two mechanical call-site updates required (`state.rs`'s `CliLoginStdin` field type, `ipc.rs`'s dispatch table).

#### `lib.rs` (1948 lines) — **SPLIT (light, higher-risk)**
`pub fn run()` alone is ~960–1690 lines (estimates varied slightly between passes depending on where cfg-gated macOS blocks were counted) — the entire CEF host bootstrap sequence, phase-commented but not phase-functioned, with several strictly-ordered steps (macOS sandbox init *before* framework load, DLL search path setup, etc.). One clean, zero-risk extraction: the macOS ObjC runtime-patch block (~710 lines: `patch_nsapp_unrecognized_selector`, `disable_macos_drag_slideback`, dock-icon/activation-policy helpers) — self-contained, each already fully doc-commented as an independent workaround, zero shared state with `run()` beyond being called from it. Proposed: new `macos_compat.rs`. **The rest of `run()` should be treated as a separate, larger follow-up** (extract phase functions *within* the file first, bundling the many threaded locals into a context struct) — not part of a first-pass split, given the explicit ordering-hazard comments throughout.

#### `agentmux-launcher/src/saga/mod.rs` (1858 lines) — **KEEP-AS-IS (production); mechanical test move optional**
The `SagaCoordinator` engine (~490-line impl: `spawn_saga`/`apply_action`/`cancel_all_in_flight`/`claim_terminal`) is genuinely cohesive — tightly coupled around shared in-flight-saga state, not a bundle of unrelated concerns. Individual sagas are *already* correctly modularized (`pool_respawn.rs`, `window_cleanup.rs`). The one real finding: **`#[cfg(test)] mod tests` spans ~768 of 1858 lines (41%)**, inline rather than in a sibling `tests.rs` the way `reducer/tests.rs` already does elsewhere in this same codebase. Moving just the test module is a zero-risk, purely mechanical win.

#### `agentmux-cef/src/client/lifecycle.rs` (1181 lines) — **KEEP-AS-IS (with an important caveat)**
One `impl AgentMuxHandler` with 5 `LifeSpanHandler` trait methods, two of which (`on_after_created` ~427 lines, `on_before_close` ~509 lines) are each **one function**, not multiple bundled concerns — extensively comment-justified as a strictly-ordered teardown/setup sequence (mutex/reducer ordering, UI-thread deadlock avoidance, launcher round-trip races). A file split alone would relocate these two giant functions without reducing their actual complexity; the higher-value fix is internal decomposition into named private helpers, which is a different, riskier exercise than this pass's scope.

#### `agentmux-launcher/src/supervisor/windows.rs` (1020 lines) — **KEEP-AS-IS (needs a precursor refactor to safely split)**
`run_windows` (~960 lines) is a single `tokio::select!`-driven supervised-wait loop mutating ~10–15 local variables across its arms (crash-restart budgets, OOM classification, splash respawn), with comments citing specific live incidents (stdin-EOF-kills-srv race, Win32 event-name collisions, recycle-kill flag ordering). Already-separable pieces (`mem_supervisor`, `srv_spawner`, `saga`, `host_spawn`) are correctly delegated elsewhere — this file is the orchestration glue left over, matching the "sequential main-flow" non-candidate pattern. **If a split is wanted later**, it requires first bundling the loop's mutable locals into a context struct (a genuine precursor refactor) — passing 8–10 raw parameters per extracted arm-handler is a readability wash, not a win.

### 2.2 Remaining SPLIT candidates (compact)

| File | Lines | Summary |
|---|---|---|
| `ui_tasks/window.rs` | 1920 | File's own header already admits it's "leftover of a prior split, never subdivided further" — ~23 independent `wrap_task!` structs with no shared state beyond `Arc<AppState>`. Split by category the header already names: `lifecycle.rs`/`geometry.rs`/`alpha.rs`/`create.rs`/`devtools.rs`/`focus_reclaim.rs`/`diagnostics.rs`. Lowest-urgency of the "organizational, not correctness" splits — current grouping isn't wrong, just large. |
| `browser_panes.rs` | 1480 | `BrowserPaneManager` is a **zero-field unit struct** — every method takes `&Arc<AppState>` as a parameter, so its ~913-line `impl` splits across files with zero shared-state risk (Rust allows multiple `impl` blocks per type). Split by method cluster: lifecycle/navigation/zoom/clip-and-focus. |
| `launcher_ipc.rs` | 1421 | Platform-triplicated connection setup (Windows/Unix/stub, ~800 lines, inherently large) vs. a family of ~18–20 uniform, stateless `report_*` functions (~440 lines) that just build a `Command` and send on `COMMAND_TX`. Extract the reporters to `launcher_ipc/reporters.rs`; `COMMAND_TX` needs `pub(crate)` visibility or an accessor fn. |
| `app.rs` | 1281 | Four `wrap_*!` CEF-delegate macro blocks (legitimately large FFI boilerplate) plus several standalone platform-utility functions with zero dependency on the delegates (GPU-tier detection, monitor/DPI geometry, Linux window-properties override) that only happen to be *called from* the delegate bodies. Extract `app/gpu.rs`, `app/monitor.rs`, `app/window_settings.rs`. |
| `agentmux-launcher/src/splash_mac.rs` | 1205 | Pure, platform-agnostic startup-stage formatting/model logic (`apply_event`/`flatten_rows`/`format_ms`, ~250 lines, already has its own dense test suite with zero Cocoa FFI) vs. the native Cocoa splash window itself (~950 lines, legitimately one FFI-glue concern). Extract `splash_mac/stage_rows.rs`. |
| `agentmux-launcher/src/ipc/server.rs` | 1140 | `ServerCtx` + accept-loop (platform-gated bind helpers) vs. one large generic `handle_connection<S>` protocol loop (~485–500 lines, genuinely cohesive per-connection state machine) vs. a self-contained 270-line `enforce_register_first` exhaustive `Command` match (a clean "big match table" extraction on its own). Split into `ipc/server/{accept,connection,register_gate,wire}.rs`. |
| `commands/window/creation.rs` | 987 | Live user-driven window creation/URL resolution (~600 lines) vs. crash-restart window **reprojection** (`reproject_from_snapshot`/`reproject_from_srv`, ~340 lines) — a clearly distinct concern (session-restore vs. live open-window request), already has its own separate test module. Extract `commands/window/reproject.rs`, matching the sibling-module pattern (`lifecycle`/`motion`/`chrome`/`transparency`/`meta`) this directory already uses. |
| `commands/providers.rs` | 942 | Four groups: config load/detect, provider auth, CLI install — plus a **generic file-copy utility block** (`copy_file_to_dir`/`copy_recursive`/`deconflict_path`, ~100 lines) that has nothing to do with providers and should move to a general-purpose location, not just its own `providers/` submodule. |
| `wrr/win_event.rs` | 911 | Core WinEventHook install/callback (~600 lines after extraction, legitimately one Win32-FFI concern) vs. a last-window quit-decision subsystem (`should_quit_on_last_window`/`arm_quit_watchdog`/etc., ~300 lines incl. its own already-embedded test module) that's fully decoupled aside from calling `app_state()`. Extract `wrr/quit_watchdog.rs`. |
| `floating_pane.rs` | 881 | Floater HWND registry + tear-off task (~360 lines) vs. the raw Win32 popup primitive (`create_popup`/`floating_pane_wndproc`, ~300 lines, shared by two consumers) vs. pane-*pool* window spawning (~140 lines) that explicitly does *not* use the floater registry per its own doc comment — a distinct feature only colocated because it reuses the popup primitive, arguably belongs in `commands/window_pool.rs` instead. |
| `client/wndproc.rs` | 528 (post-cleanup; was reported as larger before PR #2267 trimmed the dead frameless-resize cluster) | 4 independent Win32 subclass hooks (focus-restore, floater-cascade, taskbar/icon, close-routing) sharing only the common subclassing idiom, not state. Small enough to defer, but the natural split (`focus_restore.rs`/`floater_cascade.rs`/`taskbar.rs`/`close_routing.rs`) is already obvious if it grows further. |

### 2.3 KEEP-AS-IS (with reasoning)

| File | Lines | Why |
|---|---|---|
| `reducer/mod.rs` (cef) | 1225 | Already the correct pattern: 8 handler submodules (`browsers`/`drag`/`pane_pool`/`pane_window`/`panes`/`pool`/`quit`/`top_level`) do the real work; `mod.rs` holds only the shared `HostState`/`HostCommand`/`HostEvent` vocabulary + a thin `update()` dispatcher. Splitting the enums would fragment the one source of truth every submodule depends on. |
| `client/mod.rs` | 236 (confirmed via `wc -l`, far below the original estimate) | Already the "root of an already-modularized directory" — only shared constants, `mod`/`use` wiring, and the `AgentMuxHandler` struct + constructor. Nothing left to extract. |
| `agentmux-launcher/src/ipc/server.rs`'s `handle_connection` in isolation, and `commands/window/creation.rs`'s core — both noted above as KEEP within otherwise-SPLIT files (see §2.2 detail). |

---

## 3. Frontend — agent-pane subsystem (`frontend/app/view/agent/`)

15 files scanned, cross-validated by two independent passes.

### 3.1 Highest-value candidates (detailed)

#### `agent-view.tsx` (1490 lines) — **SPLIT (modest — most easy extraction already happened)**
Already delegates ~20 concerns to `./hooks/*`/`./flows/*` per `SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md`. What's left: fork-switch/fork-create handlers (~150 lines) → `useForkTabStripActions.ts`; startup-sequence RPC assembly (~70 lines) → `useAgentStartupSequence.ts`; a turn-just-ended edge detector (~40 lines, already isolated by its own long doc comment) → `useTurnJustEnded.ts`. **Do not touch** the pane-registration block (lines 332–456) — it carries an explicit comment forbidding extraction because it must run synchronously in component body, before any hook's `onMount` dispatches. The ~380-line JSX tail is markup wiring ~20 already-extracted subcomponents, not bundled logic — leave as-is.

#### `agent-model.ts` (974 lines) — **SPLIT**
`AgentViewModel` class (viewIcon/viewName/launch orchestration, genuinely one cohesive ViewModel, ~600–680 lines) vs. ~295–370 lines of **pure, zero-coupling helper functions** at the file's bottom: `buildConfigFiles`, `expandTemplate`, `buildSettingsWithHooks`, `buildMcpConfig` (CLAUDE.md/.mcp.json/.claude/settings.json synthesis — no `this`, no RpcApi calls, no SolidJS), plus `checkNodejsForProvider`/`agentmuxHome`/`resolveCliDir`. Proposed: `agent-config-builder.ts` (~240–300 lines) + optionally `agent-launch-env.ts` (~70 lines). Zero circular-import risk (config-builder has no dependency back on the class).

#### `useAgentStream.ts` (951 lines) — **SPLIT WITH CAUTION (documented crash history)**
Bundles tool-chunk streaming, persistent-shell streaming (a fully separate feature per its own spec reference), a shared RAF-batched flush core, and the main NDJSON→DocumentNode turn-lifecycle loop. **The file's own comments cite a real production crash** (`RETRO_REPLACECHILD_CRASH_2026-06-06.md`, replaceChild/reconcileArrays) caused by two independent `documentAtom` writes triggering interleaved reactive flushes in the same browser task — the single-RAF/single-`batch()` design is the fix. A split that gives each producer (tool-chunk, shell) its own RAF/`batch()` call would **silently reintroduce that exact bug**. Correct approach: extract `stream-flush-queue.ts` *first*, as the one shared sink every producer pushes into (single `requestAnimationFrame`, single `batch()`), then the individual event-source subscriptions (`useToolChunkStream.ts`, `useShellNodeStream.ts`, `useTurnLifecycle.ts`, `usePendingMessageAcceptance.ts`) can safely split around it. This is the highest-risk split in the entire scan if done naively — flag prominently to whoever implements it.

#### `AgentDocumentVirtualList.tsx` (832 lines) — **KEEP-AS-IS (mostly), same crash-history caveat**
Confirmed already heavily pre-modularized (`anchor.ts`, `streaming-buffer.ts`, `state.ts`, `expansion-source.ts`, `renderers.ts`, `DocumentRow.tsx`, `perf-probe.ts` are all already separate sibling files). What remains is the genuine virtualization algorithmic core: deliberately-plain (non-signal) `let` state so mutation doesn't retrigger reactive re-runs mid-computation, scroll handling that must read *live* refs (not captured values) to avoid a documented anchor-restore bug (#1101), and comments citing at least 8 distinct historical incident fixes. Two narrow, optional extractions exist (`useAgentPaneLayoutFeed`, `useRowMeasurement`, ~155 lines combined) if the team wants them, but the core scroll/partition logic should not be split given the fragility already documented in-file.

#### `AgentLaunchModal.tsx` (987 lines) — **SPLIT**
Extract the "Continue / New" mode logic (`continueOfId`/`isContinue`/`continueLocksIdentity`/lock flags, ~130 lines) → `useContinueOrNewMode.ts`, and auth-gating logic (`authBlocksLaunch`/`accountSupplies`/`selectedAccountStatus`, ~70 lines) → `useLaunchAuthGate.ts`. Leaves ~700–780 lines (setup/handlers + a large form's JSX — the JSX portion is markup-driven, lower priority to split further). The `flow` reactive store (from `createLaunchFlowStore`) must be threaded by reference into any extracted hook, not destructured.

#### `identity-view.tsx` (793 lines) — **SPLIT (backed by a real, present-day consumer mismatch)**
Two independent concerns — `AccountsTab`+`AccountRow`+`AccountDetail` (view/browse, ~265–320 lines) vs. `AccountForm` (create/edit, ~470–480 lines) — share only the `IdentityViewModel` prop type. **Confirmed via grep**: `AgentIdentityPanel.tsx` already imports *only* `AccountForm` from this file, meaning a real consumer today pulls in the entire 793-line module (including view/list/detail-modal code it never uses) just to reach the form it needs. Proposed: `identity-account-form.tsx` + `identity-accounts-tab.tsx`, updating the two real call sites.

#### `providers/index.ts` (633 lines) — **SPLIT (lowest-risk in this batch)**
Type contracts (~95–110 lines), a static per-CLI config table (`PROVIDERS`, ~365–390 lines — long because it's 9 CLIs' worth of hand-curated, heavily-rationale-commented data, not tangled logic, and should stay one file even after splitting out from the rest), and a **dynamic model-catalog overlay** (`modelOverlay` signal + version-comparison helpers, ~115–120 lines, real testable logic unrelated to *what a provider is*). Proposed: `providers/types.ts` + `providers/catalog.ts` + `providers/model-overlay.ts`, with `providers/index.ts` shrinking to a ~25–30-line barrel so all 18 external consumers keep their existing import path unchanged.

### 3.2 Remaining SPLIT candidates (compact)

| File | Lines | Summary |
|---|---|---|
| `AgentFooter.tsx` | 833 | Two fully independent components share a file by convention only: `AgentWorkingRow` (status/spinner row, ~200–220 lines, zero shared state) and `AgentFooter` (composer). Move `AgentWorkingRow.tsx` out — clean win. A secondary split *inside* `AgentFooter` (history recall / autocomplete / voice-handle sub-hooks) is possible but must preserve a documented event-dispatch asymmetry (`setComposerValue` deliberately skips an `input` event to avoid resetting history-recall state; the voice handle deliberately does dispatch one) — do this part with care, not as a quick win. |
| `AgentPicker.tsx` | 829 | Clean win: `useAgentDefinitions`/`useOpenDefinitionMap` (already cross-imported by `identity-view.tsx` today) → `hooks/useAgentDefinitions.ts`. Template-select flow (install → prereq → create-from-template, ~180–350 lines) is a defensible second extraction. **Also**: `openLaunchModal` and `autoContinue` are dead code (defined, never called/exported) — see §0 cross-cutting findings. |
| `stream-parser.ts` (narrow) | 690 | `ClaudeCodeStreamParser`'s core event→node state machine is genuinely cohesive. One clean seam: jekt-block security parsing (`tryParseJekt`/`parseJektTagFields`/`stripJektEnvelope`, ~120–140 lines) depends on the class only via `currentAgentId` + an id-generator callback, both trivially passable as parameters. Extract `jekt-parser.ts`; the id-generation callback must be threaded through to preserve the documented live/history-parser determinism contract. |
| `useAgentControllerStatus.ts` | 642 | Already follows the file directory's established `flows/`-vs-`hooks/` convention for its `startLaunchFlow` path — extend the same pattern to the three recovery actions (`relogin` ~150 lines, `useGlobalLogin` ~75, `loginViaTerminal` ~70) plus shared helpers → `flows/run-recovery-login.ts` (or `hooks/useAgentAuthRecovery.ts`). Not a true state decoupling (both halves need nearly the same parameter surface) but each becomes independently reviewable. |
| `PreLaunchAuthPanel.tsx` | 733 | `startConnect` (~205–215 lines) is a pure async orchestration function with zero JSX — extract to `flows/start-connect.ts`, matching the sibling convention this file already uses for `runProviderLogin`/`registerSeededAccount`. 5 small presentational sub-panels (`ConnectCta`/`WaitingPanel`/`ReadyBanner`/etc.) are a lower-priority, optional second split. |

### 3.3 KEEP-AS-IS (with reasoning)

| File | Lines | Why |
|---|---|---|
| `auth-state.ts` | 876 | Single pure reducer for one 5-`kind`/~13-command finite-state machine. Length is ~60% hard-won regression-history comments (many `reagent P1/P2 on #NNN` citations cross-referencing sibling cases), not bundled logic — splitting types from the switch would relocate lines without reducing coupling, since every case's doc comment explains conditions enforced elsewhere in the same switch. |
| `auth-flow-controller.ts` | 609 | `AuthFlowController`'s ~450-line core has every method reading/writing the same `actionToken`/`pollHandle`/`state()` invariants, with methods explicitly cross-referencing each other's guard coverage in comments. One small, optional, genuinely zero-risk extraction exists (`AuthRpc` interface + `defaultAuthRpc` adapter, ~65–95 lines, already designed for DI/testing) — low value given the file is otherwise borderline-length already. |
| `types.ts` | 747 | Pure type-declaration file (large discriminated unions + icon-map constants). No logic, no reactive coupling — this is the "exhaustive type file" non-candidate the audit brief anticipated. A feature-domain split is mechanically possible but would multiply import lines across every consumer for zero runtime benefit. |

---

## 4. Frontend — general (28 files)

Cross-validated by two independent passes; verdicts agreed in all cases.

### 4.1 Highest-value candidates (detailed)

#### `frontend/app/store/global.ts` (884 lines) — **SPLIT**
A "global grab-bag" already self-documented with section-comment dividers (`// --- Block creation`, `// --- Tab management`, `// --- Telemetry`, etc.) — evidence these are already recognized as separate concerns not yet extracted, unlike `backendStatus.ts`/`block-atom-cache.ts` sitting alongside it, which this same file already re-exports for backward compatibility (i.e. there's a proven, working precedent for exactly this kind of split in this exact file). Proposed: `block-layout-actions.ts`, `block-component-registry.ts`, `wave-file.ts`, `conn-status.ts`, `flash-notifications.ts`, `tab-actions.ts`, `dev-counters.ts`, `misc-utils.ts` — all re-exported from `global.ts` so the 97 existing import sites and the `window.globalAtoms` surface don't need to change.

#### `frontend/app/tab/tabbar.tsx` (1366 lines) — **SPLIT**
Five weakly-coupled concerns: core render/select/close (~300 lines), reorder drag-and-drop (~150–300 lines), tear-off RPC orchestration (`requestTearOff`/`tearOffTabAtRelease`, ~190–260 lines, barely touches Solid signals), cross-window tear-off event listeners (Phase 4/5 IPC handlers, ~330 lines), and active-tab color-line measurement (~90–140 lines). Proposed: `tab-tearoff.ts`/`tab-tearoff-rpc.ts`, `tab-reorder.ts`/`tab-drag-reorder.ts`, `tab-color-line.ts`, `tab-close-confirm-modal.tsx`. Shared drag/insertion-point state already lives module-level in the sibling `tabbar-dnd.ts`, so extraction needs no new prop-drilling.

#### `frontend/app/view/swarm/swarm-model.ts` (1252 lines) — **SPLIT**
Interface/type block (~200 lines), a cluster of pure functions the file's own comments already describe as extracted "so it's directly unit-testable" (`buildDispatchBuckets`/`buildShellRows`/`buildCronRows`/`mergeSubagentsPreservingIdentity`/etc., ~380 lines), a dispatch-activity-feed subsystem (~140 lines), and the `SwarmViewModel` class (~630 lines, genuinely cohesive — many small buckets sharing caches and one subscription lifecycle). Proposed: `swarm-types.ts` + `swarm-tree-builders.ts` + `swarm-dispatch-detail.ts`, leaving the class in `swarm-model.ts`.

#### `frontend/app/element/modal.tsx` (709 lines) — **SPLIT**
Already section-comment-delineated into 4 pure, zero-SolidJS-reactivity algorithms bolted onto the `Modal` component: a module-level modal *stack* (scope-containment reachability, ~60–65 lines), a *region lock* manager (WeakMap-keyed inert/scroll-lock reference counting, ~55–60 lines), generic focus-trap DOM utilities (~20–65 lines — codebase's first, no existing consolidation target), and a backdrop-dismiss animation helper (~30 lines). Proposed: `modal-stack.ts`, `modal-region-lock.ts`, `modal-focus-trap.ts`, `modal-dismiss-nudge.ts`, `modal-parts.tsx` (Header/Body/Footer), `confirm-modal.tsx`. Each becomes independently unit-testable without mounting a full component — today they can't be tested in isolation at all.

#### `frontend/layout/lib/TileLayout.{win32,linux,darwin}.tsx` (1015 / 922 / 907 lines) — **SPLIT (moderate) + cross-cutting dedup finding**
Assessed individually per the task's own constraint (no merging across platforms), each is already ~9 clearly-bounded components glued by a shared `layoutModel` prop and module-level drag globals. **Additionally** (see §0): `NodeBackdrops`, `MagnifiedPaneOverlay`, `DisplayNodesWrapper`, and `Placeholder` are byte-for-byte identical across all three files — extracting a shared `tilelayout-shared.tsx` removes ~750 duplicated lines without touching the genuinely platform-specific `DisplayNode`/`OverlayNode`/`ResizeHandle` drag logic (WebView2 vs. WebKitGTK vs. WKWebView quirks). Win32 is otherwise the closest to a legitimate KEEP given the real Windows-specific complexity dominating its remaining length (WebView2 `draggable` quirks, Win11 dragend-swallowing safety nets).

### 4.2 Remaining SPLIT candidates (compact)

| File | Lines | Summary |
|---|---|---|
| `keymodel.ts` | 707 | Debug-file-logger, focus/tab-navigation helpers, new-block/split helpers, the keymap dispatch engine, and one ~250-line `registerGlobalKeys()`. `keymodel-{debuglog,nav,blockcreate,dispatch,bindings}.ts`. `globalKeyMap`/`globalChordMap` need export (not private) so the bindings file can populate them. |
| `settings-view.tsx` | 812 | Generic form primitives + 5 independent, prop-less section components (Appearance/WindowPanes/Terminal/Sounds/Advanced) already delimited by banner comments. One file per section + `settings-controls.tsx`. Zero coupling risk — mainly serves navigability given the length is JSX-driven. |
| `browser-view.tsx` | 797 | Address bar/navigation, native-pane HWND rect syncing, loading-spinner fade, a Linux-only freeze-frame workaround (~150 lines, self-contained), and an HTTP auth-challenge queue+modal — each independently stateful. `use-pane-rect-sync.ts`/`use-freeze-frame.ts`/`use-browser-auth.ts`/`browser-nav-bar.tsx`. Real complication: freeze-frame reads rect-sync's output, so hook construction order matters; `onMount`/`onCleanup` interleaving must be preserved carefully during the split. |
| `editor-model.ts` | 1208 | Tab/content lifecycle (cohesive, ~700–850 lines) vs. file-tree filesystem mutations (`renameFile`/`deleteFile`/`createFile`/etc., ~230–250 lines, only weakly coupled to tab lifecycle) vs. pure utilities (`detectLanguage`, `sniffUnopenable`, ~120 lines combined, zero coupling). `editor-language.ts` + `editor-content-sniff.ts` (clean) + `editor-file-tree-ops.ts` (needs an injected `openFilePreview` callback). |
| `app-init.ts` | 1096 | Three fully self-contained feature installers (`installFloatingRedockHoverListener` ~160–165 lines, `installWindowTitleEffect` ~76–80, `installVoiceInputErrorListener` ~48–50) with zero cross-calls into the bootstrap core. `app/init/{floating-redock-hover,voice-input-error-listener}.ts`; window-title effect can move alongside the existing `util/window-title.ts` it already imports from. **Caution**: don't also extract `initHostWave`/`initHostNewWindow` into a separate module — they call back into `initWaveWrap`, which would create a real circular import; leave those four functions together. |
| `floating-pane-workspace.tsx` | 1056 | Edge-resize (~125–140 lines, fully self-contained) cleanly extracts to `floater-edge-resize.ts`. Header-drag + cross-window redock (~600–700 lines) is one genuine ~20–25-variable interactive state machine spanning 3 platforms' differing drag-delivery mechanisms — extractable as *one* hook/module, but resist decomposing further; only the terminal `tryRedockAtCursorInner` RPC call (~100 lines) is cleanly separable within it if desired. |
| `blockframe.tsx` | 961 | ~10 weakly-coupled UI pieces (header context-menu builders, small header buttons, `ConnStatusOverlay` ~115 lines self-contained, `BlockMask` ~48–50 lines self-contained, generic header-element renderers) around one frame wrapper. Split into `blockframe-header.tsx`/`blockframe-connstatus.tsx`/`blockframe-mask.tsx` + existing `pane-color-menu.ts`/`pane-actions.ts` absorbing the two menu builders. |
| `drone-view.tsx` | 945 | Toolbar, geometry helpers, node-inline field editors, agent-ref picker, run panel are all comment-delineated and cleanly separable; `Canvas` (~430 lines, pan/zoom/drag/wire) is one legitimately cohesive interactive system built on several `let`-scoped mutable closures sharing DOM refs — leave it as one component. `dragKind` module-level signal needs a shared home (`drone-drag-state.ts`) since both the toolbar and canvas touch it. |
| `editor-view.tsx` | 934 | LSP lifecycle (~150–220 lines) and file-tree context-menu construction (~90–130 lines, nearly fully self-contained) are conceptually unrelated to "keystroke → CodeMirror → save." Real complication: LSP hook and CodeMirror setup both need to reach into `cmView`/`lintCompartment` — extraction needs a small getter/setter interface contract, not a free lift. |
| `editor-pane-state-store.ts` | 901 | Unlike its sibling slices (`agent-pane-state/`, `browser-pane-state/`), this one hasn't yet separated types/reducer/slot-store into distinct files the way the codebase's own established convention does elsewhere. `editor-pane-types.ts` + `editor-pane-reducer.ts` (keep the switch itself intact) + slot-store remainder. |
| `keymodel.ts`'s sibling `action-widgets.tsx` | 687 | Config helpers (pure), `MoreDropdown` (self-contained floating-UI component), and the main `ActionWidgets` component bundling responsive-collapse measurement + drag-to-reorder + more-dropdown wiring. `action-widgets-config.ts`/`more-dropdown.tsx`/`use-widget-bar-responsive.ts`/`use-widget-drag-reorder.ts`. |
| `InstancePanel.tsx` | 664 | `OpacityControl` subcomponent (fully self-contained, ~55 lines) and naming-resolution pure functions (~85–90 lines) are clean wins. Inline-rename state machine (~70–110 lines) is tightly coupled to row JSX handlers — a `useRowRename` hook is possible but the payoff is marginal; defensible to leave in-file. |
| `drone-model.ts` | 653 | `DroneViewModel` bundles run-lifecycle/RPC orchestration (~230 lines) and local canvas graph-editing (`addNode`/`removeEdge`/`canConnect`/`validate`, ~148–160 lines, independently unit-testable — zero Solid/RPC dependency). `drone-graph-ops.ts` as near-pure functions taking `setDraftStore`/`draftAtom` as parameters, mirroring the existing pattern in `layoutTree.ts`. |
| `markdown.tsx` | 620 | Mermaid rendering (~75–85 lines, fully self-contained), code-block rendering (~60–65 lines), media resolution (~90 lines, async `resolveRemoteFile`/`resolveSrcSet` — a distinct concern from rendering), small element renderers. All prop-driven leaves with no shared state — `markdown-mermaid.tsx`/`markdown-code-block.tsx`/`markdown-media.tsx`. |
| `browser-model.ts` | 607 | Borderline — constructor dominated by 4 near-identical IPC-subscription blocks with pane-specific race-condition handling documented inline. Optional: convert to named private methods (`_subscribeTitleChange()` etc.) for readability; not a genuine file-boundary win since all four route through the same `_dispatch`/`diag` machinery. |

### 4.3 KEEP-AS-IS (with reasoning)

| File | Lines | Why |
|---|---|---|
| `agent-pane-state/reducer.ts` | 1087 | Single pure `update()` over one discriminated union; cases extensively cross-reference each other's invariants (turn-phase state machine, stream-subscription gate, watchdog timers) in comments. Splitting a single exhaustive switch would break TS exhaustiveness-checking in one place and scatter genuinely interdependent reasoning. |
| `browser-pane-state/reducer.ts` | 805 | Same "pure slice reducer" pattern, already in a dedicated slice folder matching the codebase's established one-reducer-per-slice convention. Helper functions are shared across most cases — splitting by case would just require re-importing (or duplicating) the same helpers everywhere. |
| `agent-document/reducer.ts` | 741 | Same pattern again. One large shared helper (`scrubOrphanedInProgress`, ~95 lines) is invoked from four different cases with subtly-tuned, PR-review-history-documented semantics — moving it to its own file is a defensible minor extraction, but the reducer itself stays one cohesive switch. |
| `layoutModel.ts` | 888 | Already the *product* of a prior successful split — nearly every method is a one-line delegator to an already-extracted implementation module (`layoutTree.ts`, `layoutResize.ts`, `layoutFocus.ts`, etc.). One legitimately non-delegated ~80-line method (`getPlaceholderTransform`) could move for consistency, but that's polish, not a structural fix. |
| `layoutTree.ts` | 642 | Already-separated, per-action-type pure mutator functions dispatched centrally from `layoutModel.ts`'s reducer — the "Redux case-functions" pattern already correctly applied. `computeMoveNode` (~195 lines, the largest single piece) is an optional, non-required extraction. |
| `termwrap.ts` | 788 | `TermWrap` follows an explicitly documented strict 3-phase lifecycle (CONSTRUCT→INIT→RUNNING) where ordering is load-bearing (race conditions from past bugs cited inline). Two narrow, low-risk optional extractions exist (WebGL renderer selection, a Windows PSReadLine workaround) but aren't a priority given the marginal size and real sequencing risk. |
| `TileLayout.win32.tsx` (assessed standalone) | 1015 | Genuinely dominated by real Windows-specific complexity (see §4.1) beyond what the shared-component extraction removes — closest of the three platform files to a legitimate KEEP once the cross-platform duplication (§0) is addressed separately. |

---

## 5. Cross-cutting findings (beyond per-file line count)

1. **`TileLayout.{win32,linux,darwin}.tsx` duplication** (§0, §4.1) — ~750 lines of byte-identical code across 3 files. Highest ratio of "lines removed" to "risk" of any single finding in this scan; doesn't require touching platform-specific drag/resize logic at all.
2. **Dead code in `AgentPicker.tsx`** — `openLaunchModal` and `autoContinue`, confirmed via grep to be unreferenced anywhere. Flag for a follow-up deletion pass (outside this scan's modularization scope, but found during it).
3. **`compute_and_ensure_bundle_dir` in `identity_handlers.rs`** (§1.1) — explicitly documented as vestigial (bundle mode retired per `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` Phase 4c). Worth confirming truly dead and deleting alongside — or instead of — moving it.
4. **Regression-history warnings requiring careful handling, not mechanical moves** — three files carry explicit in-code warnings tying their current single-file/single-batch/single-function shape to a specific past production incident: `useAgentStream.ts` (replaceChild/reconcileArrays crash), `AgentDocumentVirtualList.tsx` (same crash class + a separate anchor-restore bug, #1101), and `identity/resolver.rs` (a silently-orphaned security invariant, INV-A). Any implementation of the proposed splits for these three **must** preserve the documented invariant explicitly — this is the single most important thing for whoever picks this report up to internalize before touching those files.
5. **Test-file bloat repeats a lot** — `reducer.rs` (93% tests), `saga/mod.rs` (41% tests, inline rather than in a sibling file like this same codebase's own `reducer/tests.rs` convention), `identity/resolver.rs` (57% tests), `subprocess.rs` (18% tests but zero coverage of its two largest methods) all follow the same shape: production code already reasonably scoped, bloated by an un-externalized or undifferentiated test module. Moving tests to mirror the production split is uniformly the lowest-risk fix available in this report.

---

## 6. Suggested prioritization

Not all 62 SPLIT verdicts are equally worth doing. If picking a starting subset:

**Tier 1 — mechanical, zero-risk, high line-count impact (do these first):**
- `reducer.rs` test-file split (~3750 lines moved)
- `server/service/workspace.rs` / `server/service/window.rs` (cleanest match-arm extraction in the scan)
- `state.rs` auxiliary-type extraction (no coupling to `AppState`)
- `TileLayout.*.tsx` shared-component dedup (~750 lines, zero platform-logic risk)
- `global.ts` (proven precedent already exists in the same file)
- `providers/index.ts`, `modal.tsx` (pure-logic extractions, easy to unit-test afterward)

**Tier 2 — real value, moderate care needed (do these once Tier 1 is comfortable):**
- `subagent_watcher.rs`, `subprocess.rs`, `identity_handlers.rs`, `window_pool.rs`, `browser_panes.rs`, `launcher_ipc.rs`, `app.rs`, `platform.rs` (all mechanical extractions with clearly-named seams, just larger)
- `agent-model.ts`, `AgentLaunchModal.tsx`, `identity-view.tsx`, `keymodel.ts`, `action-widgets.tsx`, `markdown.tsx`, `tabbar.tsx`, `settings-view.tsx`, `browser-view.tsx`

**Tier 3 — genuine value but requires reading and respecting an in-code incident warning before touching (do these last, deliberately, one at a time):**
- `useAgentStream.ts` (build the shared flush queue first)
- `identity/resolver.rs` (keep the gate function + its tests together as one unit)
- `main.rs`, `lib.rs`'s `run()` (heavy sequential state threading; `lib.rs`'s macOS block is safe, the rest needs its own follow-up)
- `client/lifecycle.rs`, `supervisor/windows.rs` (both flagged KEEP-AS-IS pending a precursor context-struct refactor — don't force a split without it)

Everything else in this report (the ~40 remaining SPLIT files) is organizational value at low, uniform risk — reasonable to pick up opportunistically alongside other work in the same file, rather than as a dedicated pass.
