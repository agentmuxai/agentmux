# AgentMux — Codebase Architecture & Refactor Audit

**Date:** 2026-06-18
**Author:** smike
**Scope:** Whole repo — frontend app structure, the agent-pane subsystem, the `agentmux-srv`
Rust backend, and cross-cutting contracts / provider pipeline / workspace boundaries / docs.
**Method:** Four parallel deep-dive audits, each citing `file:line` evidence, synthesized here.
**Status:** Analysis (no code change). Companion to the just-merged
[`ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md`](ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md)
(PR #1542) and [`ANALYSIS_AGENT_WORKING_STATE_AND_INPUT_REDUCER_2026_06_17.md`](ANALYSIS_AGENT_WORKING_STATE_AND_INPUT_REDUCER_2026_06_17.md).

---

## 0. TL;DR

The codebase is a **Wave Terminal fork** whose Go backend was rewritten in Rust. It is not
"spaghetti" in the sense of tangled call graphs — most module boundaries are actually clean and
acyclic at the crate level. The debt is concentrated in **six recurring patterns**, almost all
traceable to the same root cause: **contracts and mirrors that are kept in sync by reviewer
diligence instead of by a compiler, codegen step, or test.** When the Go→Rust rewrite dropped the
TS-from-Go code generator without replacing it, that pattern metastasized.

The six systemic themes (each appears in 2–4 of the four audits):

1. **Hand-synced contracts with zero enforcement** — the FE↔BE RPC wire contract, the
   `AgentAtoms ⇄ AgentPaneState` mirror, the provider registry's 4 sync points, the mcp/bashwrap
   HTTP DTO mirrors. *The single highest-value cluster.*
2. **God-modules / god-files** — `global.ts`, `agent-view.tsx` (1282 LOC), `service.rs` (2892 LOC,
   one 2272-line match), `websocket.rs` (2371 LOC), `rpc_types.rs` (2427 LOC).
3. **Duplicated logic / near-clones** — the "is busy?" derivation repeated **17×**, three near-clone
   block controllers, four near-copy translator tool-call mappers, data-dir resolution in 3+ places.
4. **Parallel / competing systems** — four agent-pane state systems, two backend ingress dispatchers,
   dual scroll/expansion state, `specs/` vs `docs/specs/`.
5. **Dead / vestigial surface** — the dead watchdog family (StreamStalled now removed in #1542;
   submit/interrupt timeouts still dead), ~70 dead FE commands + ~66 unregistered backend constants,
   a 733-file doc graveyard.
6. **Layering violations** — `global.ts ⇄ wos.ts` circular dependency, leaf utilities importing the
   store, `srv` bypassing the canonical `DataPaths`, shared blockfile I/O buried in `shell.rs`.

**The single highest-value finding** (§2, area-4 P1): the FE↔BE RPC contract lost its generator and
is now three large hand-synced files (`rpc_types.rs` 2427 LOC, `gotypes.d.ts` 2297 LOC,
`rpc-api.ts` 1568 LOC) with **no codegen, no contract test, no CI check** — only two prose "Keep in
sync" comments. It is *already* drifting: ~70 FE command methods resolve to no handler and ~66
backend command constants are never registered. A cheap CI test that asserts command-name set parity
is the best value/effort ratio in the repo, and the *same* untyped-mirror disease recurs at the
mcp/bashwrap boundary — so one principle ("the wire contract is generated or tested, never
hand-mirrored") fixes the top finding in three places at once.

---

## 1. Prioritized cross-cutting roadmap

Ranked by value ÷ (effort × risk). IDs are referenced from the per-area sections.

| # | Action | Value | Effort | Risk | Sources |
|---|--------|-------|--------|------|---------|
| **A1** | **RPC contract enforcement.** Cheapest first: a test asserting `register_handler` names ⊇ the `rpcCall` names the FE uses, flagging dead constants both ways. Stretch: regenerate `gotypes.d.ts` + `rpc-api.ts` from `rpc_types.rs` + the registry. | ★★★★★ | Low (test) / Med (codegen) | Low | area 4 P1 |
| **A2** | **Extract srv↔mcp↔bashwrap HTTP DTOs + WPS envelope into `agentmux-common`.** Kills ~22 untyped wire mirrors; makes the two clients compile-coupled to srv. | ★★★★ | Med | Low | area 4 P2 |
| **A3** | **Break the `global.ts` god-module + `global.ts ⇄ wos.ts` cycle.** Split by concern; invert the leaf→store imports. Foundational for all frontend work. | ★★★★ | Med-High | Med | area 1 P1/P2 |
| **A4** | **Split `service.rs::dispatch_service` (2272-line match) into per-service modules**, mirroring the clean `rpc/engine.rs` registry. Behavior-preserving routing move. | ★★★★ | Med | Low | area 3 #2 |
| **A5** | **Extract a `BlockControllerCore`** shared base for the 3 near-clone controllers (status trio, spawn, session-id capture, exit finalize, health watchdog). Removes 300–400 LOC and unifies divergent lifecycle paths. | ★★★★ | Med-High | Med | area 3 #1 |
| **A6** | **Collapse the agent-pane's 4 parallel state systems**; kill the `AgentAtoms ⇄ AgentPaneState` 1:1 mirror so adding a field is a 1-file change, not 4. | ★★★★ | High | Med-High | area 2 R1–R3 |
| **A7** | **Shared translator tool-call/tool-result core** (field-map + 2 builders). Removes 4 near-copies (~150 LOC); makes "tool name didn't resolve" a one-place fix. | ★★★ | Low | Low | area 4 P3 |
| **A8** | **Split `websocket.rs`** by command family (editor / LSP / shell-exec / agent-input) following the delegation pattern the file already uses. | ★★★ | Med | Low | area 3 #4 |
| **A9** | **De-dup the agent-pane "is busy?" selector** (17 copies → one `isPaneBusy()`), and route the 11 raw `dispatch*` calls in `agent-view` through `paneModel`. | ★★★ | Low | Low | area 2 R4/R6 |
| **A10** | **Consolidate data-dir resolution onto `DataPaths`**; reconcile `AGENTMUX_DATA_HOME` (srv) vs `AGENTMUX_DATA_DIR` (launcher). | ★★★ | Med | Med | area 4 P4 |
| **A11** | **Use the real `BlockRegistry`** instead of the hardcoded map in `block.tsx`; make `ModalLayer` registry-driven instead of importing 9 concrete modals. | ★★★ | Low-Med | Low | area 1 P4/P5 |
| **A12** | **Dead-code sweep.** Finish the watchdog family (submit/interrupt timeouts, after #1542); delete the ~66 unregistered RPC constants + ~70 dead FE command methods once A1 can prove they're dead. | ★★ | Low-Med | Low | area 2, area 4 |
| **A13** | **Spec/doc hygiene.** Add `docs/specs/INDEX.md` (subsystem → authoritative spec); merge `retro`/`retros` + `analysis`/`analyses`; archive superseded per-incident specs. | ★★ | Med | Low | area 4 P6 |
| **A14** | **Shared FE event-name constants** (typed union for `WaveEvent.event`, mirrored from `wps.rs:22-54`). Folds into A1. | ★★ | Low | Low | area 4 P5 |
| **A15** | **Harden srv error handling.** Poison-tolerant RPC mutex; guard the 3 `child.std*.take().unwrap()`; surface the ACP `let _ = tx.try_send` handshake drops. | ★★★ | Low | Low | area 3 #7 |

**Sequencing.** A1 is the keystone — do it first; it de-risks A2, A12, A14 and gives the whole
team a guardrail. A4/A5/A8 (backend splits) are independent and low-risk — good parallel work. A3
and A6 are the deepest frontend changes — schedule them when there's room to absorb churn. A7, A9,
A11, A15 are cheap wins that can land anytime.

---

## 2. The single highest-value finding (detail)

**The frontend↔backend RPC contract is 100% hand-maintained with no automated enforcement.** Both
contract files say so in their own headers:

- `frontend/types/gotypes.d.ts:4-6` — *"Hand-maintained type bindings. Keep in sync with
  agentmux-srv… The original Go generator (cmd/generate/main-generatets.go) was removed with the Go
  backend."*
- `frontend/app/store/rpc-api.ts:4-6` — same note.
- `agentmux-srv/src/backend/rpc_types.rs:5` — *"Rust equivalents of Go structs from pkg/wshrpc."*

**Quantified drift surface:**

- The FE sends **202** distinct commands (`rpcCall("…")` in `rpc-api.ts`); the Rust RPC engine
  registers **~144** handlers (148 `register_handler` calls across `engine.rs` + 8 `*_handlers.rs`
  modules + `websocket.rs`). Intersection ≈ 131.
- **~70 FE command methods have no backend handler.** Three were verified end-to-end (`blockinfo`,
  `fileread`, `createblock`): they exist only as `COMMAND_*` constants in `rpc_types.rs:179-187`, are
  unregistered, unhandled in `cef`, and the corresponding `*Command` types are **never called
  anywhere in the FE either**. *Nuance:* this gap is mostly **dead Wave-inherited surface** (the
  `conn*`, `wsl*`, `remotefile*`, generic `file*` subsystems never reimplemented in Rust), not live
  broken calls — accumulated vestigial cruft, not a field of runtime landmines. **But nothing tells
  you which is which.**
- **~66 `COMMAND_*` constants in `rpc_types.rs` are never registered**, and **13 backend handlers
  have no FE method** (`agent.define/list/open/output/send/status/stop`, `pane.open`, plus test
  stubs).
- **Struct shapes are currently fine** — six spot-checked shared structs have field parity including
  correctly-mirrored serde renames (`Block`, `RpcOpts`, `BlockInfoData`, `FullConfigType`,
  `WaveEvent`, `WSEventType`). The risk isn't today's field rot; it's that **nothing prevents the
  next rename from shipping a silent runtime break.**

**Drift prevention: none.** No codegen, no contract test, no CI step (grep of
`Taskfile.yml`/`package.json`/`scripts/` for `gotypes`/`rpc_types`/`generatets` → zero;
`.github/workflows` cover input-handler sync and release-version consistency but never the RPC
surface). The guardrail is two prose comments.

The same disease recurs **inside the workspace** at the mcp/bashwrap boundary (§5b): `bashwrap`'s
`PublishRequest` (`agentmux-bashwrap/src/wps_client.rs:31-50`) is byte-identical to srv's
`WpsPublishRequest` (`server/mod.rs:441-463`), and `mcp` hand-builds ~13 endpoint bodies as
`serde_json::json!` literals (`agentmux-mcp/src/main.rs:356-782`) mirroring srv structs — with **zero
compile coupling** because neither client depends on `agentmux-common`.

---

## 3. Area 1 — Frontend app structure

**Structure.** SolidJS app under `frontend/app/` with `store/`, `view/`, `element/`, `block/`,
`layout/`, `util/`, `tab/`, `modals/`. The layering intent (leaf `util`/`layout` → `store` → `view`
→ `element`) is sound but violated in several places.

**Smells (evidence):**

- **CRITICAL — `global.ts ⇄ wos.ts` circular dependency.** `store/global.ts:39` imports from
  `wos.ts`; `store/wos.ts:12` imports back from `global.ts`. A bidirectional store cycle is the
  hardest kind to reason about and the easiest to deadlock during init ordering.
- **`global.ts` is a god-module** — **87 exports, 95 importers**, and it imports *upward* into
  `@/app/modals` and `@/app/tab` (a store reaching into views). It's the single most central file in
  the FE and the natural blast radius for any change.
- **Leaf-layer violations (14 reach-ins):** `util/logger.ts:4` and `layout/lib/layoutModel.ts:4-7`
  import the store — leaf utilities should never know about app state.
- **`element/ModalLayer.tsx:55-64`** imports **9 concrete agent-view modals** directly, instead of a
  registry — every new modal edits this file (A11).
- **Hardcoded `BlockRegistry` map** in `block/block.tsx:47-61` shadows the real registry — a second
  source of truth for block types (A11).
- **`rpc-api.ts` is 1,568 LOC of hand-maintained ex-generated code** (see §2).
- **Inconsistent view shapes** and **flat god-stores vs. reducer-dirs** — some subsystems use the
  clean `update(state, cmd) → {state, events}` reducer pattern (agent-pane), others are flat mutable
  stores; no consistent convention.

**Proposals:** A3 (break the cycle + split `global.ts` by concern — selection, layout-glue,
object-cache, modal-glue), A11 (registry-drive blocks + modals), invert the leaf→store imports.

---

## 4. Area 2 — Agent-pane subsystem

The agent-pane has the **best** core (the pure `update()` reducer with the events/EventSink fan-out)
and the **worst** accumulation around it.

**Smells (evidence):**

- **`agent-view.tsx` god-file — 1282 LOC.** Mixes rendering, dispatch, scroll, and derivation.
- **The "is busy?" derivation is duplicated 17×.** `status.isLoading() || workingFromPhase(turnPhaseAtom())`
  appears verbatim in 17 sites. One `isPaneBusy()` selector replaces all (A9).
- **FOUR parallel state systems** for one pane: the `turnPhase` reducer state, the `AgentAtoms`
  mirror, the per-pane `documentState`, and the `agent-pane-layout-store`. Adding one field can mean a
  **4-file change**.
- **`AgentAtoms ⇄ AgentPaneState` is a 1:1 mirror** — the reducer state is copied into atoms for
  rendering; the two must be kept in sync by hand (A6).
- **Dual scroll/expansion state** — `documentState.scrollPosition`/`pinnedNodes` vs. layout store
  `scrollTop`/`expansion`, bridged by `expansion-source.ts`. Two sources of truth for "where is the
  user scrolled / what's expanded."
- **3 generations of dispatch helpers** coexist; **11 raw `dispatch*` calls in `agent-view` bypass
  `paneModel`**, the intended single entry point (A9).
- **6 translators with no base class** (~150 collapsible LOC) — see area 4 §5a for the shared-core
  proposal (A7).
- **The dead-watchdog family is larger than just `StreamStalled`.** `schedule-submit-timeout`
  (`reducer.ts:343`) and `schedule-interrupt-timeout` (`reducer.ts:568`) follow the *same*
  emitted-but-never-dispatched pattern. **PR #1542 removed only the `StreamStalled` member** (the one
  explicitly scoped); the submit/interrupt timeouts remain dead and should be removed in a separate,
  explicitly-scoped change (A12).

**Proposals:** A6 (collapse the 4 state systems / kill the mirror — the deepest win), A9 (de-dup the
selector + route through `paneModel`), A7 (translator base), A12 (finish the watchdog sweep).

---

## 5. Area 3 — `agentmux-srv` Rust backend

**Structure.** ~24k in-scope LOC. The egress layering (Broker → EventBusBridge → per-conn EventBus
lanes, `eventbus.rs:221-249`) is **genuinely clean and tested** — a model to copy. The problems are
the two *ingress* dispatchers and a cluster of god-files + controller duplication.

**Largest modules:** `server/service.rs` 2892, `server/app_api.rs` 2853, `backend/rpc_types.rs`
2427, `server/websocket.rs` 2371, `backend/blockcontroller/shell.rs` 2358,
`blockcontroller/subprocess.rs` 1813, `blockcontroller/persistent.rs` 1181.

**Smells (evidence):**

- **`service.rs::dispatch_service` — the worst god-function: ~2272 lines, one `match`**
  (`service.rs:275-2547`, 46 arms across 9 services; the WorkspaceService section alone is
  `1046-2407`). Each arm inlines arg-parsing + business logic (A4).
- **Two parallel ingress dispatchers with confirmed overlap.** `service.rs:2519` literally annotates
  it: *"App API (also reachable via WebSocket RPC in app_api.rs)"* — `("agent","define")` calls the
  *same* `app_api::agent_define_core` the WS RPC handler calls. The `*_core` business logic is
  correctly factored; the **routing is duplicated** across a giant HTTP match and a WS registry, so
  verbs can drift between the two surfaces.
- **`websocket.rs` god-file (2371 LOC)** mixes transport (connection lifecycle, protocol mux) with
  ~40 inline handlers — `COMMAND_AGENT_INPUT` ≈ 280 lines (`:944-1224`), `COMMAND_SHELL_EXEC` ≈ 200
  lines (`:1263-1463`), 12 editor/file-ops handlers (`:1541-1959`), 3 LSP handlers. It *also*
  correctly delegates 8 families to sub-modules (`:2256-2348`) — proving the pattern works; the rest
  just never got extracted (A8).
- **Three near-clone block controllers (~300–400 LOC duplicated).** `persistent.rs` / `subprocess.rs`
  / `acp.rs` (+`shell.rs`) duplicate: the status trio (`set/get/publish_status`, byte-identical), the
  spawn boilerplate (`~`-expand + `create_dir_all` + env loop), the session-id capture→persist→broadcast
  block (~35 lines, the biggest), exit/kill finalization, and a copy-pasted health watchdog loop. The
  magic string `"agent:sessionid"` is re-typed in every reader (A5).
- **`shell.rs` god-file** — a single ~700-line `start()` (`:365-1057`), plus **shared blockfile I/O
  free functions** (`extract_agent_events`, `handle_append_block_file`, `mirror_append_to_global`,
  `rebuild_output_idx`, `:1209-1563`) that the *other* controllers import via `super::shell::…` —
  shared infra mislocated inside the shell file (extract to `blockcontroller/blockfile_io.rs`).
- **`rpc_types.rs`** — 2427-line DTO dump with `#![allow(dead_code)]` at line 1, 177 flat
  `COMMAND_*` consts; already section-bannered, so a `rpc_types/` submodule split is mechanical.
- **Error-handling risks:** 15 `self.inner.lock().unwrap()` in the RPC engine (`engine.rs:214…530`)
  — one panicking handler poisons the mutex and cascades; silent RPC drops (`engine.rs:364,510`);
  unguarded `child.std*.take().unwrap()` (`subprocess.rs:491`, `persistent.rs:665`, `acp.rs:270`) that
  panic the `start` thread despite a `Result` signature; the ACP handshake `let _ = tx.try_send`
  cluster (`acp.rs:388,503-505,542-543,554`) that can silently never-start an agent (A15).
- **Provider registry has 4+ sync points** (`providers.rs:367-438,494`) — static def, registry
  insert, a separate hardcoded order array, the aliases map, and a `== 9` count test all edited per
  provider. `agent_config.rs` is also Claude-shaped (hardwires `CLAUDE.md`/`.claude/…` paths).

**Clean spots (copy these):** the Broker→Bridge→EventBus egress; the data-driven `rpc/engine.rs`
registry (the model `service.rs` should adopt); the `HistoryAdapter` trait
(`history/adapter.rs:87-100`); `process_tracker` as correctly-shared infra.

**Proposals:** A4 (split the god-match), A5 (`BlockControllerCore`), A8 (split `websocket.rs`),
A15 (harden errors), + extract `blockfile_io.rs` and data-drive the provider registry.

---

## 6. Area 4 — Contracts, provider pipeline, workspace, docs

**(a) Provider/translator pipeline — 4 near-copies of one mapping.** Five translators implement
`OutputTranslator` (`providers/translator.ts:10`). Claude is legitimately unique (390 LOC, stateful
Anthropic streaming). The duplication is the **`tool_call` / `tool_result` mapping**, copy-pasted
across **codex, gemini, kimi, acp**: each independently declares `toolNameById: Map<string,string>`
(`codex:31`, `gemini:25`, `kimi:23`, `acp:25`), builds a `ToolCallEvent` (`codex:124-143`,
`gemini:68-80`, `kimi:51-68`, `acp:48-60`), and a `ToolResultEvent` (`codex:145-157`, `gemini:82-95`,
`kimi:75-95`, `acp:62-75`) — ~60–70% mechanically identical, differing only in **field names**
(`call_id`/`tool_id`/`toolCallId`; `arguments`/`parameters`/`input`). Proposed `tool-correlation.ts`
core: the shared map + two field-map-parameterized builders (`makeToolCall`, `makeToolResult`); each
translator shrinks to a field-map + its genuinely-unique routing. Cuts ~150 LOC, low risk (A7).

**(b) Rust workspace boundaries.** Dependency direction is **clean and acyclic** — `common` is a
leaf; `srv`/`cef`/`launcher` depend only on `common`; the IPC `Command`/`Event` protocol is correctly
centralized in `agentmux-common::ipc` and not redefined. Two real smells:
- **`mcp` and `bashwrap` depend on nothing shared** yet are HTTP clients of srv's `/api/v1`, so srv's
  request contract is re-declared client-side with zero compile coupling (~22 hand-maintained
  mirrors; `bashwrap`'s `PublishRequest` is a byte-copy of `WpsPublishRequest`, then re-wrapped into a
  *third* copy `WaveEvent`) (A2).
- **Data-dir resolution exists in 3+ places and srv bypasses the canonical one.**
  `agentmux-common/src/data_paths.rs:4-7` is explicitly *"the single source of truth."* But srv has
  its own (`base.rs:62-154`) plus ad-hoc `dirs::home_dir().join(".agentmux")` at
  `config_watcher_fs.rs:41`, `main.rs:698-701,1293-1295`, `history/index.rs:19`,
  `history/claude_adapter.rs:62`. Worse, srv honors `AGENTMUX_DATA_HOME` while the launcher exports
  `AGENTMUX_DATA_DIR` — **the two resolvers can disagree on where data lives** (A10).

**(c) WPS/wave-event names duplicated as untyped strings.** Rust centralizes them
(`wps.rs:22-54`: `EVENT_BLOCK_FILE="blockfile"`, …); the FE types the field as plain `string`
(`gotypes.d.ts:1670,1743`) and scatters ~25 bare event-name literals across 21+ files with no shared
constant module. A typo on either side fails silently (A14).

**(d) Docs — a 733-file graveyard.** `docs/specs/` = 373, `docs/analysis/` = 95, top-level `specs/` =
90 (+ `specs/archive/` 27). Duplicate dir names: both `docs/retro/` **and** `docs/retros/`; both
`docs/analysis/` **and** `docs/analyses/`. Per-incident proliferation (e.g. two competing
window-drag designs dated the same day; separate macOS/Linux/Windows tearoff specs). The
`docs/specs` (draft) → `specs` (approved) → `specs/archive` lifecycle **stalled** (approved tree
newest 2026-06-15 while drafts grow to 06-17). **No index of record.** Demonstrable drift: execution
plans reference `saga.rs` / `saga_coordinator.rs` that no longer exist (A13).

---

## 7. What's already healthy (don't "fix")

To keep the refactor honest — these are good and should be the templates:

- The agent-pane **`update()` reducer** + events/`EventSink` fan-out (pure, tested, auditable).
- The srv **egress** chain: Broker → EventBusBridge → per-conn EventBus lanes (`eventbus.rs:221-249`).
- The srv **`rpc/engine.rs`** data-driven handler registry (what `service.rs` should become).
- The **Rust crate dependency graph** — acyclic, `common` as a clean leaf, IPC protocol centralized.
- The **`HistoryAdapter`** trait and `process_tracker` shared infra.

The refactor strategy is therefore mostly **"make the rest of the codebase look like the parts that
already work"** — adopt the registry pattern, the reducer pattern, and (above all) replace
hand-synced mirrors with generated-or-tested contracts.

---

## 8. Reference index (entry points for each action)

- **A1** — `frontend/app/store/rpc-api.ts:4-6`, `frontend/types/gotypes.d.ts:4-6`,
  `agentmux-srv/src/backend/rpc_types.rs:5,179-187`, the `register_handler` sites in
  `engine.rs` + `server/*_handlers.rs`.
- **A2** — `agentmux-bashwrap/src/wps_client.rs:31-50`, `agentmux-srv/src/server/mod.rs:441-463`,
  `agentmux-srv/src/backend/wps.rs:72-82`, `agentmux-mcp/src/main.rs:356-782`.
- **A3** — `frontend/app/store/global.ts:39`, `frontend/app/store/wos.ts:12`,
  `frontend/util/logger.ts:4`, `frontend/layout/lib/layoutModel.ts:4-7`.
- **A4 / A5 / A8** — `agentmux-srv/src/server/service.rs:275-2547`,
  `…/blockcontroller/{persistent,subprocess,acp,shell}.rs`, `…/server/websocket.rs:944-2348`.
- **A6 / A9** — `frontend/app/view/agent/agent-view.tsx`, `…/store/agent-pane-state/`,
  `…/agent-pane-layout-store.ts`, `…/expansion-source.ts`.
- **A7** — `frontend/app/view/agent/providers/{codex,gemini,kimi,acp,claude}-translator.ts`.
- **A10** — `agentmux-common/src/data_paths.rs:4-7,271`, `agentmux-srv/src/backend/base.rs:16-154`.
- **A11** — `frontend/app/block/block.tsx:47-61`, `frontend/app/element/ModalLayer.tsx:55-64`.
- **A12** — `frontend/app/store/agent-pane-state/reducer.ts:343,568` (submit/interrupt watchdogs);
  `agentmux-srv/src/backend/rpc_types.rs` unregistered `COMMAND_*` consts.
- **A13** — `docs/specs/`, `specs/`, `docs/retro(s)/`, `docs/analys(i/e)s/`.
- **A15** — `agentmux-srv/src/backend/rpc/engine.rs:214-530`,
  `…/blockcontroller/{subprocess.rs:491,persistent.rs:665,acp.rs:270,388,503-554}`.
