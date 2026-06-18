# Architecture Refactor — Tracking & Handoff (A1–A15)

**Created:** 2026-06-18 · **Owner of record:** smike · **Status:** living tracker
**Source audit:** [`ANALYSIS_CODEBASE_ARCHITECTURE_AUDIT_2026_06_18.md`](ANALYSIS_CODEBASE_ARCHITECTURE_AUDIT_2026_06_18.md)
(read it first — this doc is the actionable board on top of it; the audit holds the full
file:line evidence and the six systemic themes.)

> **For the next agent.** This is a self-contained handoff. Pick any 🟢 item, read its entry below
> (entry points, approach, acceptance criteria, gotchas), follow the **Workflow** in §3, and update
> the status board in §1 + the item's status line in your PR. Each item is sized to one PR. **Before
> you start any item, re-run the fleet-conflict check in §3.4** — the blockers below were true on
> 2026-06-18 and will change as other agents' PRs merge.

---

## 1. Status board

Value/Effort/Risk are from the audit. "Gate" = which trees the PR touches (collision surface).

| # | Action | V | E | R | Status | PR | Notes / blockers |
|---|--------|---|---|---|--------|----|------------------|
| A1 | RPC contract enforcement test | ★★★★★ | Low | Low | ✅ **done** | #1544 | `test/contract/rpc-contract.test.ts`. Keep baselines shrinking. |
| A2 | Extract srv↔mcp↔bashwrap HTTP DTOs into `agentmux-common` | ★★★★ | Med | Low | 🔴 **blocked** | — | Collides with a5af **#1498** (`agentmux-mcp`). Wait for it to merge. |
| A3 | Break `global.ts` god-module + `global.ts ⇄ wos.ts` cycle | ★★★★ | Med-High | Med | 🟢 ready | — | Frontend `store/`. Big; do when there's room for churn. |
| A4 | Split `service.rs::dispatch_service` (2272-line match) | ★★★★ | Med | Low | 🟢 ready | — | Backend `server/`. Largest god-function. Mechanical. |
| A5 | Extract `BlockControllerCore` (3 near-clone controllers) | ★★★★ | Med-High | Med | 🟢 ready | — | Backend `blockcontroller/`. Lifecycle-critical. |
| A6 | Collapse agent-pane's 4 parallel state systems / kill the mirror | ★★★★ | High | Med-High | 🔴 **blocked** | — | Collides with AgentU **#1543** (agent-pane reducer/store/types). |
| A7 | Shared `ToolCorrelator` for translator tool-call/result | ★★★ | Low | Low | ✅ **done** | #1545 | `providers/tool-correlation.ts`. |
| A8 | Split `websocket.rs` by command family | ★★★ | Med | Low | 🟢 ready | — | Backend `server/`. Follows the file's own delegation pattern. |
| A9 | De-dup agent-pane "is busy?" selector (17×); route via `paneModel` | ★★★ | Low | Low | 🔴 **blocked** | — | Same gate as A6 (#1543). |
| A10 | Consolidate data-dir resolution onto `DataPaths` | ★★★ | Med | Med | 🟢 ready | — | Backend; touches where live data lives — migration care. |
| A11 | Real `BlockRegistry` + registry-driven `ModalLayer` | ★★★ | Low-Med | Low | 🟢 ready | — | Frontend `block/`, `element/`. |
| A12 | Dead-code sweep (watchdog family; dead RPC constants) | ★★ | Low-Med | Low | 🟡 **partial** | #1542 | StreamStalled removed; submit/interrupt watchdogs + ~66 RPC consts remain. |
| A13 | Spec/doc hygiene (`INDEX.md`, merge dup dirs, archive) | ★★ | Med | Low | 🟢 ready | — | Docs only. |
| A14 | Shared FE event-name constants (typed `WaveEvent.event`) | ★★ | Low | Low | 🟢 ready | — | Folds into A1's theme; pairs well after A1. |
| A15 | Harden srv error handling (mutex poison, `take().unwrap()`, ACP drops) | ★★★ | Low | Low | 🟢 ready | — | Backend; cheapest safety win. |

**Legend:** ✅ done · 🟡 partial · 🟢 ready (no known conflict) · 🔴 blocked (active fleet PR).

**Suggested next pulls (no conflicts, high value):** A4 → A8 → A5 (backend god-files / dup), or
A15 (cheap safety), or A14 (rides A1's momentum), or A11 (frontend, isolated).

---

## 2. Per-item detail

Each item: **entry points** (where to start), **approach**, **acceptance criteria** (what "done" is),
**gotchas**. Done items record what landed + lessons.

### A1 — RPC contract enforcement ✅ (#1544)
- **Landed:** `test/contract/rpc-contract.test.ts`. Re-derives both contract sides from source and
  freezes three shrink-only baselines: `liveUnregistered` (commands the FE *calls* with no handler),
  `declaredUnregistered` (rpc-api.ts methods with no handler), `registeredUndeclared` (handlers with
  no FE binding). A new drift fails the test.
- **Live latent gaps it documented** (13, the actionable subset for follow-up): `activity`,
  `connconnect/conndisconnect/connensure/connlist/connlistaws/wsllist` (conn/wsl), `deletesubblock`,
  `fileappend`, `filejoin`, `recordtevent`, `resolveids`, `setrtinfo`. Wiring or deleting any of
  these shrinks the baseline (good).
- **Lessons (read before extending it):**
  1. `register_handler(...)` takes **both** a string literal (`"readeditorfile"` on the next line)
     **and** a `COMMAND_*` const. The extractor must resolve both — the first draft only did consts
     and over-reported 28 false gaps. `registered` = 144.
  2. `rpc-api.ts` bindings use **both** `rpcCall` and `rpcStream`. An unbounded `…*?rpcCall` regex
     skips `rpcStream`-only methods and mismaps neighbors — Codex caught this. The extractor now
     bounds each method to `[its signature, the next signature)`. `declared` = 211.
  3. Sanity guards (`registered>120`, `declared>180`, a `setmeta` round-trip, a `fileliststream`
     stream check, "no unresolved binding") prevent a broken regex from passing vacuously. Keep them.
- **Stretch (not done):** regenerate `gotypes.d.ts` + `rpc-api.ts` from `rpc_types.rs` + the registry
  (true codegen). Bigger; the test is the cheap guardrail.

### A2 — Extract srv↔mcp↔bashwrap HTTP DTOs into `agentmux-common` 🔴
- **Entry points:** `agentmux-bashwrap/src/wps_client.rs:31-50` (`PublishRequest`, byte-copy of srv's
  `WpsPublishRequest` at `agentmux-srv/src/server/mod.rs:441-463`, re-wrapped into a 3rd copy
  `WaveEvent` at `backend/wps.rs:72-82`); `agentmux-mcp/src/main.rs:356-782` (~13 endpoint bodies as
  `serde_json::json!` literals mirroring srv structs like `ShellCreateRequest` `server/mod.rs:486-499`
  and `rpc_types::CommandPaneOpenData` `rpc_types.rs:922-947`).
- **Approach:** move the `/api/v1` request/response DTOs + the WPS envelope into `agentmux-common`;
  have srv/mcp/bashwrap import them. `mcp`/`bashwrap` currently depend on **nothing shared** (add
  `agentmux-common` to their `Cargo.toml`). ~22 hand-maintained mirrors collapse to one
  compiler-checked contract.
- **Acceptance:** the DTOs exist once in `agentmux-common`; `cargo check` across the workspace is
  green; mcp/bashwrap no longer redeclare srv request shapes.
- **Gotcha / blocker:** a5af **#1498** edits `agentmux-mcp/src/main.rs`. **Do not start until #1498
  merges** (or coordinate). Re-check open PRs (§3.4).

### A3 — Break `global.ts` god-module + cycle 🟢
- **Entry points:** `frontend/app/store/global.ts:39` ↔ `frontend/app/store/wos.ts:12` (bidirectional
  import cycle). `global.ts` = 87 exports, 95 importers, imports *up* into `@/app/modals` + `@/app/tab`.
  Leaf violations: `frontend/util/logger.ts:4`, `frontend/layout/lib/layoutModel.ts:4-7` import the store.
- **Approach:** split `global.ts` by concern (selection / layout-glue / object-cache / modal-glue);
  invert the leaf→store imports (pass state in, don't import it). Break the wos cycle by extracting the
  shared piece both need into a third leaf module.
- **Acceptance:** no `global.ts ⇄ wos.ts` cycle (a madge/dependency-cruiser check or a simple import
  assertion); `util/` + `layout/lib/` no longer import `@/app/store`; `global.ts` materially smaller.
- **Gotcha:** highest-fan-in file in the FE — expect wide churn; land when the agent-pane gate (A6/#1543)
  is clear to avoid overlapping store edits.

### A4 — Split `service.rs::dispatch_service` 🟢
- **Entry points:** `agentmux-srv/src/server/service.rs:275-2547` — one `async fn` with a
  `match (service, method)` of 46 arms / 9 services (WorkspaceService alone is `1046-2407`).
- **Approach:** convert to a registry mirroring the clean `backend/rpc/engine.rs`, or at minimum one
  `dispatch_<service>` fn per service with `dispatch_service` reduced to a router. Behaviour-preserving.
- **Acceptance:** `dispatch_service` is a thin router; each service's arms live in their own fn/module;
  `cargo check` green; behaviour identical (the `*_core` fns already hold the logic — just move routing).
- **Gotcha:** note the **two-ingress** overlap (`service.rs:2519` "also reachable via WebSocket RPC")
  — A4 only reorganises the HTTP `WebCall` side; unifying the two dispatchers is a separate step
  (audit §5 area 3 #3).

### A5 — Extract `BlockControllerCore` 🟢
- **Entry points:** `blockcontroller/{persistent,subprocess,acp,shell}.rs`. Duplicated: status trio
  `set/get/publish_status` (byte-identical: `persistent.rs:236-259`, `subprocess.rs:196-221`,
  `acp.rs:115-138`, `shell.rs:216-219`); spawn boilerplate (`~`-expand + `create_dir_all` + env loop:
  `persistent.rs:564-620`, `subprocess.rs:402-438`, `acp.rs:209-239`); session-id capture→persist→
  broadcast (`persistent.rs:811-863`, `subprocess.rs:606-670`); exit/kill finalize; health-watchdog
  loop (copy-pasted 3×). Magic string `"agent:sessionid"` re-typed in every reader.
- **Approach:** a `BlockControllerCore` struct/module holding the shared status/spawn/session-id/exit/
  watchdog logic + a `SESSIONID_META_KEY` const; controllers keep only their transport differences
  (PTY / stream-json / ACP).
- **Acceptance:** ~300-400 LOC removed; the 3 divergent session-id/exit paths unified (subprocess is
  careful, acp is not — converge on the careful one); existing controller tests pass; `cargo check` green.
- **Gotcha:** lifecycle-critical (Med risk). Also relocate the shared blockfile I/O out of `shell.rs`
  (`extract_agent_events`/`handle_append_block_file`/`mirror_append_to_global`/`rebuild_output_idx`,
  `shell.rs:1209-1563`) into a neutral `blockcontroller/blockfile_io.rs` — other controllers import them
  via `super::shell::…` today.

### A6 — Collapse agent-pane's 4 state systems 🔴
- **Entry points:** `frontend/app/view/agent/agent-view.tsx` (1282 LOC); `…/store/agent-pane-state/`
  (the `turnPhase` reducer — the *good* core); the `AgentAtoms` 1:1 mirror; per-pane `documentState`;
  `agent-pane-layout-store.ts`. Dual scroll/expansion bridged by `expansion-source.ts`.
- **Approach:** make the reducer state the single source; render from it directly (drop the
  `AgentAtoms` mirror so adding a field is a 1-file change, not 4); unify the dual scroll/expansion.
- **Acceptance:** adding a pane state field touches one place; no `AgentAtoms ⇄ AgentPaneState` copy.
- **Gotcha / blocker:** AgentU **#1543** is editing the agent-pane reducer/types/store right now.
  **Blocked until #1543 merges.** Pairs with A9.

### A7 — Shared `ToolCorrelator` ✅ (#1545)
- **Landed:** `frontend/app/view/agent/providers/tool-correlation.ts` (`ToolCorrelator` map+`call()`+
  `result()`+`reset()`, and `wrapOutput`). codex/gemini/kimi/acp refactored to use it; **claude left
  untouched** (genuine streaming-accumulation complexity, not duplication).
- **Lessons:** behaviour was preserved *exactly* — codex `_raw`/`call-` prefix, kimi object-args +
  `{content}` shape, acp `map ?? params.toolName ?? "unknown"` (via `fallbackName`, since
  `a ?? b ?? c === a ?? (b ?? c)`). When consolidating translators, keep field extraction inline and
  only share the mechanics.

### A8 — Split `websocket.rs` by command family 🟢
- **Entry points:** `agentmux-srv/src/server/websocket.rs` (2371 LOC). Inline handlers to extract:
  `COMMAND_AGENT_INPUT` (~280 lines, `:944-1224`), `COMMAND_SHELL_EXEC` (~200, `:1263-1463`), editor/
  file-ops (`:1541-1959`), LSP (`:2180-2236`). The file **already** delegates 8 families to submodules
  at `:2256-2348` — follow that exact pattern.
- **Approach:** move the inline families into `editor_handlers.rs` / `lsp_handlers.rs` / existing
  `agent_handlers.rs`, leaving `websocket.rs` as transport + protocol-mux only.
- **Acceptance:** `websocket.rs` materially smaller (transport/mux only); handlers in family modules;
  `cargo check` green; the A1 contract test still green (registered command set unchanged).
- **Gotcha:** registrations use string-literal command names here (see A1 lesson #1) — keep names byte-identical or the A1 test catches you (that's the point).

### A9 — De-dup "is busy?" + route via `paneModel` 🔴
- **Entry points:** `status.isLoading() || workingFromPhase(turnPhaseAtom())` duplicated **17×** across
  the agent view; 11 raw `dispatch*` calls in `agent-view.tsx` bypass `paneModel`.
- **Approach:** one `isPaneBusy()` selector; route the 11 raw dispatches through `paneModel`.
- **Acceptance:** one definition of "busy"; no raw `dispatch*` in `agent-view.tsx`.
- **Gotcha / blocker:** same gate as A6 (#1543).

### A10 — Consolidate data-dir resolution onto `DataPaths` 🟢
- **Entry points:** canonical `agentmux-common/src/data_paths.rs:4-7` ("single source of truth") is
  bypassed by srv's own `backend/base.rs:62-154` + ad-hoc `dirs::home_dir().join(".agentmux")` at
  `config_watcher_fs.rs:41`, `main.rs:698-701,1293-1295`, `history/index.rs:19`,
  `history/claude_adapter.rs:62`. Env mismatch: srv honours `AGENTMUX_DATA_HOME` (`base.rs:16-22`),
  launcher exports `AGENTMUX_DATA_DIR` (`data_paths.rs:271`).
- **Approach:** route srv path resolution through `agentmux_common::DataPaths`; reconcile the two env
  var names.
- **Acceptance:** one path resolver; env vars reconciled; **migration check** that existing installs
  still find their data (Med risk — this is *where live data lives*).

### A11 — Real `BlockRegistry` + registry-driven `ModalLayer` 🟢
- **Entry points:** hardcoded `BlockRegistry` map at `frontend/app/block/block.tsx:47-61` (shadows the
  real registry); `frontend/app/element/ModalLayer.tsx:55-64` imports **9 concrete** agent-view modals.
- **Approach:** use the real block registry instead of the hardcoded map; make `ModalLayer`
  registry-driven so a new modal registers itself instead of editing the layer.
- **Acceptance:** no hardcoded block map; `ModalLayer` doesn't import concrete modals; adding a
  block/modal doesn't edit these files.

### A12 — Dead-code sweep 🟡 (partial, #1542)
- **Done:** the dead `StreamStalled` streaming-idle watchdog (#1542) — see
  [`ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md`](ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md).
- **Remaining:**
  1. The rest of the dead watchdog **family**: `schedule-submit-timeout` (`agent-pane-state/reducer.ts:343`)
     and `schedule-interrupt-timeout` (`:568`) + their `SubmitTimeoutElapsed`/`InterruptTimeoutElapsed`
     commands follow the same emitted-but-never-dispatched pattern. Verify unwired, then delete (mirror
     the #1542 approach). **Gate:** agent-pane (#1543) — coordinate.
  2. The ~66 unregistered `COMMAND_*` constants in `rpc_types.rs` and the ~70/13 dead FE-method /
     backend-only commands — A1's test already inventories these (its baselines). Delete in batches;
     each deletion shrinks an A1 baseline (update it in the same PR).
- **Acceptance:** dead symbols gone; A1 baselines shrink accordingly; tsc/cargo green.

### A13 — Spec/doc hygiene 🟢
- **Findings:** ~733 markdown files; duplicate dirs `docs/retro/`+`docs/retros/` (merged),
  `docs/analysis/`+`docs/analyses/` (merged); `specs/` vs `docs/specs/` lifecycle stalled; no index of record;
  demonstrable drift (execution plans reference `saga.rs`/`saga_coordinator.rs` that no longer exist).
- **Approach:** add `docs/specs/INDEX.md` (subsystem → current authoritative spec); merge the duplicate
  dirs; archive superseded per-incident specs.
- **Acceptance:** one index exists; no duplicate-named dirs; superseded specs archived.
- **Status:** ✅ dirs merged; `docs/specs/INDEX.md` added; all stale path refs updated.

### A14 — Shared FE event-name constants 🟢
- **Entry points:** Rust centralises WPS/wave-event names (`backend/wps.rs:22-54`,
  `EVENT_BLOCK_FILE="blockfile"`, …); FE types the field as plain `string` (`gotypes.d.ts:1670,1743`)
  and scatters ~25 bare event-name literals across 21+ files (e.g. `store/global.ts:262,284`,
  `store/wos.ts:86`, `useAgentFailure.ts:111,127`).
- **Approach:** a shared `wps-events.ts` (typed union for `WaveEvent.event`, mirroring `wps.rs:22-54`);
  replace the bare literals. Consider a small contract test (à la A1) asserting FE union ⊇ the Rust
  consts.
- **Acceptance:** one FE constants module; `WaveEvent.event` typed; literals replaced.

### A15 — Harden srv error handling 🟢
- **Entry points:** 15 `self.inner.lock().unwrap()` in `backend/rpc/engine.rs:214-530` (one panicking
  handler poisons the mutex → cascade); silent RPC drops `engine.rs:364,510`; unguarded
  `child.std*.take().unwrap()` at `subprocess.rs:491`, `persistent.rs:665`, `acp.rs:270` (panic the
  `start` thread despite a `Result` signature); ACP handshake `let _ = tx.try_send` at
  `acp.rs:388,503-505,542-543,554` (agent can silently never start).
- **Approach:** poison-tolerant mutex helper (or `parking_lot`); log the two silent drops; guard the
  `take().unwrap()` to return `Err`; surface the ACP handshake send failures.
- **Acceptance:** no `lock().unwrap()` / unguarded `take().unwrap()` on these paths; `cargo check` green.
- **Note:** cheapest safety win; self-contained.

---

## 3. Workflow & conventions (follow these exactly)

### 3.1 Verification (run before every PR)
- Frontend types: `npx tsc -p tsconfig.json --noEmit` — **note:** the tree has a pre-existing baseline
  of ~31 unrelated errors (layout/keyutil/identity/etc.). Confirm your files add **zero** new errors;
  don't try to fix the baseline.
- Tests: `npx vitest run <paths>` (root = repo root). Run the suites your change touches + the A1
  contract test if you touched RPC handlers/bindings.
- Backend: `cargo check -p agentmux-srv` (warnings are pre-existing; errors are yours).
- Changeset: `task changeset -- patch "<conventional-commit summary>"` (one per PR; `patch` for
  refactor/chore/test/docs).

### 3.2 Branch + commit
- Branch off latest `main`: `git checkout main && git pull --ff-only && git checkout -b smike/<slug>`.
  (Use your own agent prefix, not `smike/`, if you are a different agent.)
- End commit messages with: `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

### 3.3 PR + merge gate (BOTH bots must be green)
- `gh pr create` with a body that states what/why/how + verification.
- Two reviewers run automatically/on-demand:
  - **reagentx-workflow** — auto-reviews on PR open; its **APPROVED** satisfies branch protection.
  - **chatgpt-codex-connector** — trigger it by commenting **`@codex review`** on the PR.
- **Merge only when both are green:** reagent `APPROVED` + Codex either "no major issues" or all its
  inline suggestions addressed. Codex *does* find real bugs — on #1544 it caught a genuine extractor
  defect; fix and re-push before merging.
- **Pushing a new commit dismisses reagent's approval** (branch protection). Re-request via
  `gh api -X POST repos/agentmuxai/agentmux/pulls/<n>/requested_reviewers -f "reviewers[]=reagentx-workflow"`,
  wait ~60s, re-check.
- Merge: `gh pr merge <n> --squash --delete-branch`. Then `git checkout main && git pull --ff-only`.
- Mark the item ✅ in §1 + its §2 status line **in the same PR** (update this doc).

### 3.4 Fleet-conflict check (DO THIS BEFORE PICKING AN ITEM)
- `gh pr list --state open --json number,title,author,files` — see which trees are being edited.
- **Hard rule (carried from prior sessions):** never commit to, push to, rebase, or merge another
  agent's PR/branch. Only touch your own. If your item's gate (the trees it edits) overlaps an open
  PR by another agent, mark it 🔴 and pick a non-overlapping item.
- Known on 2026-06-18: a5af **#1498** → `agentmux-mcp` (blocks A2); AgentU **#1543** → agent-pane
  reducer/store/types (blocks A6/A9). Re-verify — these will change.

---

## 4. Quick reference — biggest files / hotspots (from the audit)

| File | LOC | Item |
|------|-----|------|
| `agentmux-srv/src/server/service.rs` | 2892 | A4 |
| `agentmux-srv/src/server/app_api.rs` | 2853 | (well-split already) |
| `agentmux-srv/src/backend/rpc_types.rs` | 2427 | A1/A12 (split by section banners) |
| `agentmux-srv/src/server/websocket.rs` | 2371 | A8 |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 2358 | A5 |
| `frontend/types/gotypes.d.ts` | 2297 | A1 (hand-maintained) |
| `frontend/app/store/rpc-api.ts` | 1568 | A1 (hand-maintained) |
| `frontend/app/view/agent/agent-view.tsx` | 1282 | A6/A9 |
| `frontend/app/store/global.ts` | (87 exports / 95 importers) | A3 |

**Healthy patterns to copy** (audit §7): the agent-pane `update()` reducer + `EventSink` fan-out; the
srv egress chain Broker→EventBusBridge→EventBus lanes (`eventbus.rs:221-249`); the data-driven
`rpc/engine.rs` registry; the acyclic crate graph with `agentmux-common` as a clean leaf; the
`HistoryAdapter` trait. The refactor thesis: **make the rest look like these, and replace hand-synced
mirrors with generated-or-tested contracts.**
