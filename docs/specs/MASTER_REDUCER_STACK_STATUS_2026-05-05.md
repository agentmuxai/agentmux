# Master Reducer-Stack Status — 2026-05-05

**Purpose.** Single point-in-time snapshot of where AgentMux's reducer architecture stands across the launcher / host / srv stack, plus the frontend consumer layer and persistence durability layer. Supersedes the scattered status retros listed in §10. **Read this first** when picking up reducer-architecture work; only descend into the cited source docs when you need detail.

**Scope.** State, not implementation plans. For active PR sequencing see [`frontend-reducer-implementation-plan-2026-05-03.md`](./frontend-reducer-implementation-plan-2026-05-03.md), [`SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md`](./SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md), and [`srv-phase-e4b-implementation-plan-2026-05-03.md`](./srv-phase-e4b-implementation-plan-2026-05-03.md).

**Authority.** When this file disagrees with another spec, the source spec wins for *design* and this file wins for *current status*. Update this file when status changes; don't fork the source specs.

> **Staleness note (2026-08-03):** this file hasn't been updated since its own 2026-05-29 correction (§0) — now ~9 weeks stale. A later, independently-scoped pass exists — `docs/specs/REPORT_REDUCER_STACK_AUDIT_2026_07_26.md` — covering the same subsystem without referencing or updating this file. Check both before trusting either as "current status"; this file's own "Authority" claim above no longer holds uncontested.

---

## 0. Code-verified refresh — 2026-05-29 (READ THIS FIRST; §3–§9 below are the 2026-05-05 snapshot, retained as history)

The original §2–§9 snapshot is **~3.5 weeks / ~280 merges stale** and materially under-reports progress. The relevant work shipped under commit subjects that don't contain its phase tags (e.g. "CPD"), so a subject-grep audit missed it. This section is **code-verified** (read the source files, not the git log); where §2–§9 disagree with §0, §0 wins.

**Headline: the reducer stack is at its practical end-state.** Almost everything §2–§9 marked *designed / pending / open* has since shipped.

### Layer status — corrected

| Layer | 2026-05-05 said | Code-verified 2026-05-29 |
|---|---|---|
| **Launcher (L1)** | ✅ done | ✅ done (unchanged) |
| **Srv reducer (Phase E)** | 🟨 mostly | ✅ **E.2c.5b + E.6 shipped** — renderer-side typed-event subscriber (`frontend/util/srv-events.ts`) + multi-source dispatcher with saga buffer (`frontend/util/event-buffer.ts`); `srv_event_bridge.rs` (PR #618). E.5.x UpdateWindowMeta through reducer (#856). |
| **Srv layout (E.4.B)** | 🟨 Phase 5 "❌ next" | 🟨 **Phase 5 SHIPPED** (`reducer/layout.rs` + `Command::Layout{Clear,SetTree,InsertNode,DeleteNode}` arms + tests). **Open: Phase 6** (persist arms — `persist_subscriber` doesn't consume `Event::Layout*`; LayoutState still persists wcore-direct) **+ Phase 7** (legacy `pendingbackendactions`/rootnode writers in `server/app_api.rs:339,798` still bypass the reducer — multi-side migration). |
| **Host reducer (Phase F)** | 🟨 6/7; H.6 dormant | 🟨 **+1 slice**: `pane_window_states` (Phase 0 #1154, Phase 1 #1157). H.6 top-level-creation still dormant by design (works via direct path). |
| **Cross-process dispatch** | ❌ open / `IssueCmd::Host` log-only | ✅ **DONE — CPD-1→5.** Schema/HostFrame (CPD-1); `host_pipe/` launcher wrapper (CPD-2); live saga dispatch via `HostPipe::send_command` (CPD-3, "no longer log-only", integration-tested); per-saga `saga_id` correlation, evict-and-replace retired (CPD-4); host-side reader + idempotency LRU `(saga_id,kind)` in `agentmux-cef/src/launcher_ipc.rs:196+` (CPD-5). Only `PipeTarget::Srv`/`LauncherSelf` remain — reserved `#[allow(dead_code)]`, **no consumer**, dormant by design (spec §5 risk 4 flags preemptive activation as rot). |
| **Sagas** | ✅ foundation; F.6 ❌ pending | ✅ **F.6 WindowCleanupCascade shipped** (`saga/window_cleanup.rs`) **+ saga durability/recovery** (`saga/recovery.rs`, beyond original scope). PoolRespawn now dispatches for real via CPD. |
| **Frontend** | 🟨 #1 shipped, #2 doc-approved, #3–#8 designed | ✅ **8 slice reducers live** (code-verified `reducer.ts` files): agent-document (#1), agent-pane-state (#4 — full turn-phase series A–G #987–#997 + InitPhase + persist + cascade hardening), browser-pane-state (#9), editor-pane-state (#10), **launcher-event (#6 — said "designed")**, drone-run-state, launch-flow-state (Stages 2a–2d), window-opacity. |
| **Persistence** | ✅ subscriber shipped | ✅ unchanged (Phase G still deferred). |

### Genuinely-open reducer-stack items (the entire remaining list)

1. **E.4.B Phase 6** — layout persist-subscriber arms (consume `Event::Layout*`; retire the wcore-direct LayoutState persist path).
2. **E.4.B Phase 7** — migrate the legacy `pendingbackendactions`/rootnode writers (`app_api.rs:339,798`) onto the layout reducer. Multi-side (frontend currently applies `pendingbackendactions`), not a trivial swap.
3. **`PipeTarget::Srv` / `LauncherSelf`** — dormant by design; build only when a class-D/E saga needs launcher→srv or launcher-self dispatch.
4. **H.6** host top-level-creation runner — dormant; low priority (direct path works).

Everything else in §3–§9 below that reads as "pending/designed/open" should be treated as **shipped** unless it's one of the four above. A full row-by-row rewrite of §3–§9 is deferred; this §0 is the authoritative current status.

---

## 1. The 3-level stack

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (renderer / SolidJS)         per-process atoms     │  consumer
│  ▲ Slice-based reducer migration in progress (8 slices)      │
└─────────────────────────────────────────────────────────────┘
              ▲ CEF JS bridge ▲ events
┌─────────────────────────────────────────────────────────────┐
│  Host (agentmux-cef)                   FFI + UI thread       │  Layer 2
│  Reducer scope: pending_window_creations, active_drag,       │
│    tear_off_hooks, lifecycle, event_version                  │
│  Scaffolding (deliberately retained): browsers, window_pool  │
└─────────────────────────────────────────────────────────────┘
              ▲ launcher → host pipe (NOT YET BUILT)
┌─────────────────────────────────────────────────────────────┐
│  Launcher (agentmux-launcher)          process & OS facts    │  Layer 1
│  Reducer scope: lifecycle, processes, windows, monitors,     │
│    pool, instance_registry, backend_window_ids, pending_hwnds│
│  WRR (Window Reality Reconciliation) via Win32 hooks         │
└─────────────────────────────────────────────────────────────┘
              ▲ launcher pipe (named pipe IPC)
┌─────────────────────────────────────────────────────────────┐
│  Srv (agentmux-srv)                    app domain            │  Layer 3
│  Reducer scope: workspaces, tabs, blocks, layouts,           │
│    windows (srv view), agents, identity_accounts             │
│  Saga coordinator lives here (Path A, decided E.5)           │
└─────────────────────────────────────────────────────────────┘
              ▲ persist subscriber (idempotent SQLite write-back)
┌─────────────────────────────────────────────────────────────┐
│  Persistence                                                  │  durability
│  - objects.db, filestore.db, sagas.db, launcher-sagas.db     │
│  - launcher-events.log (JSONL)                               │
│  - In-memory event ring (4096) + disk log (Phase D)          │
└─────────────────────────────────────────────────────────────┘
```

Each reducer is canonical for its domain. Cross-reducer state moves only through events. Multi-reducer operations are sequenced by sagas. See [`multi-reducer-proposal-2026-04-28.md`](../retro/multi-reducer-proposal-2026-04-28.md) for the full pattern.

---

## 2. Layer status at a glance

| Layer | Status | Source spec | Status retro |
|---|---|---|---|
| **Launcher** | ✅ DONE — Phases B.1–B.9 + D.1–D.3 | n/a — emerged from B work | [`phase-b-roadmap.md`](../retro/phase-b-roadmap.md) |
| **Srv (Phase E)** | ✅ MOSTLY DONE — E.1a–E.5 + F1.A/B shipped | [`SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md`](./SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md) | [`phase-e-status-2026-05-01.md`](../retro/phase-e-status-2026-05-01.md) |
| **Srv layout (E.4.B)** | 🟨 IN-FLIGHT — Phase 2/3/4 shipped, Phase 5+ pending | [`srv-phase-e4b-formal-spec-2026-05-03.md`](./srv-phase-e4b-formal-spec-2026-05-03.md) | This doc, §5 |
| **Host (Phase F)** | 🟨 6/7 MIGRATED — F.1 + H.1/H.2/H.3/H.4/H.5 fully wired; H.6 top-level creation runner dormant (no dispatchers) | [`SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`](./SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md) | [`phase-fg-status-2026-05-01.md`](../retro/phase-fg-status-2026-05-01.md) |
| **Frontend** | 🟨 IN-FLIGHT — slice #1 shipped (#681), #2 doc-approved, #3–#8 designed | [`frontend-reducer-architecture-2026-05-03.md`](./frontend-reducer-architecture-2026-05-03.md) | This doc, §6 |
| **Sagas** | ✅ FOUNDATION — coordinator + 7 sagas merged | [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) | [`saga-architecture-migration-complete-2026-05-02.md`](../retro/saga-architecture-migration-complete-2026-05-02.md) |
| **Persistence** | ✅ Persist subscriber + event log shipped | n/a | Captured in E.2c.1–5a in phase-e-status |
| **Phase G** | ⏸️ DEFERRED until Phase F.6 + cross-process dispatch land (F.7 ✅ host bridge dedup shipped #722; renderer reactive-leak root cause fixed #724) | n/a | [`phase-fg-roadmap-2026-05-01.md`](../retro/phase-fg-roadmap-2026-05-01.md) §4 |

---

## 3. Layer 1 — Launcher reducer

**Status: ✅ DONE.** Single-writer; canonical for its domain.

**Owns (all canonical, no projections elsewhere):**
- `state.lifecycle` — Starting → Running → Quitting → Dead
- `state.processes` — PID → kind/state/version
- `state.windows` — label → HWND, visible, iconic, geometry, foreground timestamp
- `state.pool`, `instance_registry`, `backend_window_ids`
- `state.monitors` — Win32 monitor topology
- `state.pending_hwnds` — orphan reconciliation queue
- `event_version` — monotonic counter

**Plumbing:**
- Named-pipe IPC server, single-instance via `first_pipe_instance(true)` bind (Phase B.6 — pipe handle is OS-owned, ERROR_ACCESS_DENIED is the canonical second-instance signal)
- Broadcast bus distributes events to host + srv + renderers
- WRR observation via `SetWinEventHook` + `WM_WINDOWPOSCHANGED` (Phase B.9; zero polling)
- 60 unit tests + property tests + close-all integration test
- `--diag wrr` tool client for cross-process observability
- Phase D durable: in-memory ring (4096 events) + disk log

**Source:** decisions logged in [`phase-b-roadmap.md`](../retro/phase-b-roadmap.md). Architecture analysis in [`b5-migration-architecture-2026-04-28.md`](../retro/b5-migration-architecture-2026-04-28.md). WRR design in [`wrr-design-2026-04-28.md`](../retro/wrr-design-2026-04-28.md).

**Open from this layer:** none. Phase B exit criteria met. Browser pool / `browsers` field deliberately retained as scaffolding (see §4.4).

---

## 4. Layer 2 — Host reducer (Phase F)

**Status: 🟨 6/7 MIGRATED.** B.5 shadow mirrors built; host's own reducer (`HostState`) holds 6 of 7 planned migration domains all fully wired to production callers. Only H.6 (top-level creation runner) remains as dormant scaffolding.

### 4.1 B.5 shadow mirrors (events-only)

The host holds **read-only projections** of launcher state, updated via `apply_shadow_projection` (extracted from `apply_event_to_shadow` in PR #711 for testability):
- `shadow_instance_registry`
- `shadow_window_meta`
- `shadow_backend_window_ids`
- `window_meta` (sync cache for `open_subwindow` parent-liveness + cascade-close child enumeration)

Idempotency contract enforced per §8.14 with property tests in `agentmux-cef/src/launcher_ipc.rs::shadow_projection_tests`.

### 4.2 Migrated to `HostState` (6 phases, fully wired)

Each followed the standard a→b→c→d→e ratchet from [`migration-pattern.md`](../retro/migration-pattern.md):

| H.x | `HostState` field | Legacy field on `AppState` | Notes |
|---|---|---|---|
| **F.1** | `pending_window_creations: VecDeque<PendingWindowCreation>` | DELETED | Original pre-H scope; landed alongside Phase B.4 |
| **H.1** | `browser_panes: HashMap<String, BrowserPaneEntry>` | DELETED (`pane::lifecycle::PaneStateMachine`) | `BrowserPaneManager` (browser_panes.rs) is now a zero-sized handle delegating all mutations through `host_dispatch` |
| **H.2** | `browsers: HashMap<String, BrowserHandle>` | DELETED | Reversed the "scaffolding indefinitely" decision from `b5-migration-architecture-2026-04-28.md` — turned out the snapshot-and-drop discipline mapped cleanly onto reducer-arm semantics |
| **H.3** | `active_drag: Option<DragSession>` | DELETED | |
| **H.4** | `pool: PoolState` | DELETED (`window_pool`, `unpromoted_pool_labels`) | Drift-storm fix (PR #708) added `just_promoted_labels` to bridge the host's `PoolPromoted → WindowOpened` IPC gap |
| **H.5** | `quit_state: QuitState` | DELETED (`is_quitting: AtomicBool`) | |

### 4.3 Scaffolded but NOT wired

| H.x | `HostState` field | Legacy code still authoritative | Estimated effort |
|---|---|---|---|
| **H.6** top-level | `top_level_creation: TopLevelCreationState` (carries `#[allow(dead_code)]`) | `ui_tasks::post_create_window` direct calls (no state to migrate; pure function-call path) | 2–3 days; lower priority — current path works |

Wiring up H.6 would make every top-level open observable via the event log (timing per phase, etc.) but doesn't fix any current bug.

### 4.4 Why "Phase H" and "Phase F" specs both exist

[`SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md`](./SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md) is the granular per-domain spec; [`SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md`](./SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md) is its 5-PR operational compression. [`SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`](./SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md) is the original framing — the H.x naming used in code came after, when the scope expanded beyond F's "pending creations + active drag + tear-off hooks." Treat them as a series: F (initial scope) → H (full host migration) → this doc for current status.

---

## 5. Layer 3 — Srv reducer (Phase E)

**Status: ✅ MOSTLY DONE.** E.1a–E.5 + F1.A/B all shipped. Smoke regression fixed.

### 5.1 What shipped

Per [`phase-e-status-2026-05-01.md`](../retro/phase-e-status-2026-05-01.md):
- **E.1a:** Saga coordinator framework in launcher (left as labeled stub; E.5 chose srv-side)
- **E.1b:** Srv reducer skeleton + broadcast bus + event log
- **E.2:** Workspace lifecycle (CreateWorkspace, DeleteWorkspace)
- **E.2b:** Tab lifecycle + ActiveTab
- **E.3:** Block lifecycle
- **E.2c.1–5a:** Persist subscriber (idempotent SQLite write-back) + RPC migration
- **E.4.A:** Layout focused/magnified reducer arms (#632)
- **E.5.1–9:** Saga foundation + tear-off/restore sagas + atomic single-step commands + final wcore migrations
- **F1.A:** Subscriber SQLite transactions (atomicity per saga step)
- **F1.B:** Frontend orphan-cleanup on hard host failure

### 5.2 E.4.B — full layout-tree mutations (this week)

Tracked separately because it's been actively shipping. Reference: [`srv-phase-e4b-formal-spec-2026-05-03.md`](./srv-phase-e4b-formal-spec-2026-05-03.md).

| Phase | What | PR | Status |
|---|---|---|---|
| 2 | Typed `LayoutNode` struct + tabN naming fix | #688 (+ #689 follow-up) | ✅ shipped |
| 3 | Layout Command/Event wire types | #690 | ✅ shipped |
| 4 | 11 pure tree-manipulation helpers (TS-oracle parity) | #691 + #692 follow-up | ✅ shipped 2026-05-05 |
| 5 | Reducer arms calling the helpers | — | ❌ next |
| 6 | Persist subscriber arms | — | ❌ |
| 7 | Migrate existing `rootnode` writers off the legacy path | — | ❌ |

**Helpers shipped in #691/#692:** `insert_node`, `insert_node_at_index`, `delete_node`, `move_node`, `swap_nodes`, `resize_nodes`, `replace_node`, `split_horizontal`, `split_vertical`, `clear_tree_node`, `find_node_by_id`. All match TS oracle (40 tests). Notable correctness work that landed:
- ID swap + flex-direction reversal in leaf promotion (TS `addIntermediateNode` parity)
- `extra` field preservation through promotion (forward-compat catch-all from #688/#689)
- Same-parent move index compensation (TS insert-then-remove vs Rust detach-then-insert)
- TS-oracle clamp + leaf-stop in `insert_node_at_index` (out-of-range/over-deep `index_arr` resolves like TS instead of erroring)

### 5.3 What's still open in srv

| Item | Why open | Source |
|---|---|---|
| **E.2c.5b** — TS renderer dispatcher (`window.__agentmux_srv_event` handler + atom routing) | Plumbing done, just needs TS wiring | phase-e-status §3 |
| **E.4.B Phase 5+** — reducer arms, persist arms, rootnode migration | Helpers ready (this week); arms not started | srv-phase-e4b-formal-spec §5–7 |
| **E.6** — full multi-source consumption + renderer saga buffer | E.2c.5b prerequisite | phase-e-status §4 |
| **E.7** — property tests + integration tests + diag tools | Not started | phase-e-status §5 |
| **Per-event saga_id correlation** | Codex flagged twice during E.5; deferred (~300 LOC). Evict-and-replace mitigates | phase-fg-roadmap §3 |
| **Cross-process saga dispatch** | `IssueCmd::Host` events log-only today — no launcher→host pipe | [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md) |

### 5.4 Sagas (srv-side per E.5)

| Saga | PR | Status |
|---|---|---|
| TearOffTab, TearOffBlock, RestoreTornOffTab | #621 | ✅ |
| DeleteBlock, DeleteTab | #633 | ✅ |
| DeleteWorkspace | #639 (F1.B) | ✅ |
| PoolRespawn | #634 (`IssueCmd::Host` log-only — no actual cross-process dispatch yet) | 🟨 |
| WindowCleanupCascade | — | ❌ pending (F.6) |

**Durability:** PR 1 shipped (#631). PR 2 — resume-on-startup + `--diag sagas` — pending per phase-fg-roadmap. See [`SPEC_SAGA_DURABILITY_2026-05-01.md`](./SPEC_SAGA_DURABILITY_2026-05-01.md).

---

## 6. Frontend reducer (slice-based)

**Status: 🟨 IN-FLIGHT.** Slice #1 shipped, #2 doc-approved, #3–#8 designed.

Per [`frontend-reducer-architecture-2026-05-03.md`](./frontend-reducer-architecture-2026-05-03.md):

| Slice | Scope | Status | Spec |
|---|---|---|---|
| #1 | agent-document — per-blockId `documentAtom` reducer (3-writers bug fix) | ✅ shipped PR #681 (v0.33.618) | [`agent-pane-document-reducer-2026-05-03.md`](./agent-pane-document-reducer-2026-05-03.md) |
| #2 | conventions — Command/event types, slot lifecycle, audit, echo-loop guard | ✅ doc-approved (no code yet) | [`frontend-reducer-conventions-2026-05-03.md`](./frontend-reducer-conventions-2026-05-03.md) |
| #3 | source-tagging + global event log (CommandSource + ring buffer) | ❌ designed, descoped per Q1+Q4 | open question §9.4 |
| #4 | agent-pane-state bundle (streaming, sessionStats, currentTool, …) | ❌ designed | architecture §4 |
| #5 | frontend-layout (mirror of srv E.4.A focused/magnified) | ❌ designed; awaits srv E.4.A soak | architecture §5 |
| #6 | launcher-event-reducer convergence | ❌ designed | architecture §6 |
| #7 | tab-state mirror (active tab, order, metadata) | ❌ designed | architecture §7 |
| #8 | pane-tree (full tree mirror) | ❌ designed; deferred pending srv E.4.B Phase 5+ | architecture §8 |

**Lesson from PR-E1 retro** ([`pr-e1-layout-reducer-already-exists-2026-05-03.md`](../retro/pr-e1-layout-reducer-already-exists-2026-05-03.md)): the layout system already has a frontend reducer (in-place mutation via `layoutModel.ts` + validators). Converging it to the new conventions is gold-plating. The actual value is multi-window focus sync (E.2 territory). **Code-inspect before writing "new reducer" specs.**

---

## 7. Persistence

**Status: ✅ Persist subscriber + event log shipped.**

- **Persist subscriber** (Phase E.2c.1–5a): idempotent apply to SQLite, wrapped in transactions (F1.A). Sole writer path for Workspace/Tab/Block state.
- **WaveObjUpdate broadcast bridge** (PR #852, [`SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`](./SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md)): a peer subscriber to `srv_events_tx` that translates reducer events into frontend `waveobj:update` WS broadcasts via the existing `event_bus`. Idempotent by construction — each broadcast is a fresh `wstore.get<T>()`, so re-delivery yields the same WaveObjUpdate (frontend dedups on `version` at `wos.ts:272`). Phase 1 covers workspace events; Phase 2 expands to tab/block/window/layout. Closes the class of bug where mutation RPCs returned `success_empty()` and the response broadcast loop had nothing to fan out.
- **SQLite files** (per memory `reference_persistence_files.md`):
  - `objects.db` — Workspace/Tab/Block/Layout/Window/Agent/Identity domain state
  - `filestore.db` — file-store cache
  - `sagas.db` — srv saga state
  - `launcher-sagas.db` — launcher saga state (currently stub)
- **JSONL:** `launcher-events.log` — Phase D durable event log
- **In-memory:** ring buffer of 4096 events for crash forensics

**Phase G** (event-sourced, drop SQLite) is deferred indefinitely pending Phase F.6+F.7 + cross-process dispatch validation. Per [`phase-fg-roadmap-2026-05-01.md`](../retro/phase-fg-roadmap-2026-05-01.md) §4: "the decision to DO Phase G at all is still open" — reducer-as-source-of-truth is validated for srv; jury still out for the full system.

---

## 8. Architectural decisions to honor

These came out of the multi-reducer proposal sessions and have been reaffirmed across phase-b-roadmap, phase-e-status, phase-fg-roadmap. **Don't relitigate without strong cause.**

1. **Three reducers, each canonical for its domain.** Launcher owns OS-level facts; host owns CEF integration; srv owns app domain. Each broadcasts events; others hold projections. ([`multi-reducer-proposal-2026-04-28.md`](../retro/multi-reducer-proposal-2026-04-28.md) §1–2)

2. **Events are the only cross-reducer contract.** No reducer mutates another's state. Idempotent `apply_event` handlers enforce. (proposal §2)

3. **Sagas for cross-reducer operations.** Multi-step flows are state machines outside the reducers, sequencing commands and waiting for events. **Srv-side coordinator** (Path A) per [`saga-coordinator-location-analysis-2026-04-30.md`](../retro/saga-coordinator-location-analysis-2026-04-30.md).

4. **Versioned events + snapshot/replay.** Subscribers detect gaps and resync. (Phase D pattern.)

5. **`browsers` + warm pool stay scaffolding indefinitely.** CEF callback access patterns are unsuitable for reducer migration. Snapshot-and-drop is the long-term answer. ([`b5-migration-architecture-2026-04-28.md`](../retro/b5-migration-architecture-2026-04-28.md), [`SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`](./SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md) §3)

6. **Single-instance lives in launcher pipe bind, not host port-file.** ERROR_ACCESS_DENIED is the canonical second-instance signal. (phase-b-roadmap B.6)

7. **WRR via Win32 hooks, no timers.** `SetWinEventHook` + `WM_WINDOWPOSCHANGED` deliver every transition synchronously. (phase-b-roadmap B.9)

8. **Pool refill suppressed during host drain via `is_quitting`.** Without it, pool windows keep state non-empty forever. (phase-b-roadmap B.9.3)

9. **Frontend ↔ launcher via host JS bridge.** Renderers stay sandboxed; host is trust boundary. (phase-b-roadmap B.7)

10. **Cross-thread work uses `cef::post_task`, not direct calls.** Direct calls from worker threads are CEF UB. (phase-b-roadmap B.9.3)

11. **`merge_meta_patch` stays as deliberate escape hatch.** Opaque-meta allows fast iteration on metadata without multiplicative reducer complexity. (phase-fg-roadmap §2.5)

12. **Phase G go/no-go deferred until F.6+F.7+cross-process dispatch.** Phase E validated the pattern for one reducer; need fuller system validation before committing to event-sourced everywhere. (phase-fg-roadmap §4.3)

13. **Doc-only PRs are noise.** `docs/retro/*.md` are local working notes; don't open PRs for them. **This master spec is in `docs/specs/` precisely because it's intended for review.** (phase-b-status memory note)

14. **Launcher event subscribers MUST be idempotent under `(event_kind, label, version)`.** Duplicates may legally arrive from re-dispatch, resync (Phase D `GetSnapshot`), or replay; the contract is that subscribers fold them into a no-op past the first application. The three subscriber sites and their property-test coverage:
    - **Renderer-side bridge guard** (`shouldDispatchLauncherEvent` in `frontend/util/launcher-events.ts`) — `(event, label, hwnd) → max_version_seen` map. Property tests in PR #709 (`launcher-events.test.ts`).
    - **Renderer-side reducer** (`frontend/app/store/launcher-event/reducer.ts`) — pure reducer; per-event-kind idempotency. Property tests in PR #709 (`reducer.test.ts`).
    - **Host shadow projection** (`apply_shadow_projection` in `agentmux-cef/src/launcher_ipc.rs`, extracted from `apply_event_to_shadow`) — `HashMap::insert`/`remove` keyed by `label`, idempotent by construction. Property tests in this PR (`shadow_projection_tests`).

    **Drift-storm motivation:** prior to PR #708 the renderer-side dispatcher was idempotent only via the tracker's global monotonic-version drop, which fails when a fresh JS context resets `lastVersion=0` (pool window bootstrap) — same launcher event re-dispatched ~600× → V8 stack exhaustion → Crashpad. See [`ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`](./ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md).

15. **`srv_events_tx` subscribers MUST be idempotent under `(event, oid, version)`.** Companion contract to §8.14 covering the srv-side reducer event bus. Each subscriber may observe the same reducer event multiple times (replay, restart re-bootstrap, future cross-process dispatch). Two subscriber sites:
    - **`persist_subscriber`** (`agentmux-srv/src/persist_subscriber.rs`): SQLite write-back. Idempotent via `INSERT OR REPLACE` + `apply_event_to_wstore` arms keyed by oid + version.
    - **`wave_obj_bridge`** (`agentmux-srv/src/server/wave_obj_bridge.rs`, PR #852): translates events → frontend `waveobj:update` broadcasts. Idempotent by construction — each call fetches fresh state from `wstore`. Frontend dedups on `version` at `wos.ts:272`, so duplicate broadcasts collapse to a single signal update.

    **Migration arc:** every Phase E command that's been migrated through the reducer (UpdateWorkspace, UpdateWorkspaceMeta, UpdateTabMeta, UpdateBlockMeta, …) emits an event onto this bus. The persist subscriber writes SQLite; the bridge propagates to the frontend. Commands NOT yet migrated (UpdateWindowMeta, UpdateLayoutMeta, …) bypass the reducer entirely and fall back to `wcore` direct writes — they'll inherit reactivity automatically when their Phase E.5.x migration lands. See PR #852's spec for the bridge architecture and Phase 2 mapping table.

---

## 9. Open questions

These are real design decisions still pending. Listed in priority order.

### 9.1 Cross-process saga dispatch — RESOLVED (CPD-1 through CPD-5 shipped)

**Status: ✅ DONE.** All five sub-PRs from [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md) §4 landed:

| CPD-N | Scope | Lands as |
|---|---|---|
| **CPD-1** | Schema additions: `saga_id` mandatory on `SpawnPoolWindow`/`ReapPanes`/`DrainPoolIfLast`; `Option<u64>` on `Report*` Commands and Events; `HostFrame` envelope; `ReportSagaActionFailed` event | `agentmux-common/src/ipc.rs` |
| **CPD-2** | `HostPipe` wrapper + framing + connection loop + bounded `pending_buffer` | `agentmux-launcher/src/host_pipe/` |
| **CPD-3** | `apply_action` dispatches via `host_pipe.send_command()` instead of log-only; `inject_saga_id` helper; per-saga `timeout()` override | `agentmux-launcher/src/saga/mod.rs` |
| **CPD-4** | Per-saga `on_event()` correlation by `saga_id`; evict-and-replace policy retired | `pool_respawn.rs`, `window_cleanup.rs` |
| **CPD-5** | Host-side `(saga_id, kind)` LRU for idempotency on duplicate launcher dispatches | `agentmux-cef/src/saga_dispatch.rs` |

F.5 (`pool_respawn_on_promote`) and F.6 (`window_cleanup_cascade`) sagas drive host-side actions through the wire instead of just narrating organic events. Saga timeouts and retries are meaningful for `IssueCmd::Host`. Concurrent same-kind sagas correlate cleanly.

**Open follow-up (deferred):** `PipeTarget::LauncherSelf` and `PipeTarget::Srv` arms are still log-only stubs (no consumer); spec §3.6 recommended preemptive wire-up; deferred until a class-D/E saga needs them. The companion durability spec ([`SPEC_LAUNCHER_SAGA_DURABILITY`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md), Phase LSD) builds on top of CPD and is its own thread.

### 9.2 Per-event saga_id correlation — RESOLVED (folded into CPD-4)

**Status: ✅ DONE.** Closed with §9.1 above. CPD-4 wires `saga.on_event()` to filter by `event.saga_id == self.expected_saga_id`; evict-and-replace policy retired. The "~300 LOC, deferred twice" estimate was the work that landed inside CPD-4.

### 9.3 Phase F hosting strategy — RESOLVED in-process

**Resolved (de-facto):** the host reducer runs in the `agentmux-cef` process (option B), with `host_dispatch` invoked synchronously from CEF callbacks and IPC handlers. The "launcher Tokio centralisation" alternative was never built. F.1 + H.1/H.2/H.3/H.4/H.5 all shipped under this model without cross-process coordination issues.

**Cross-process bridging** is still needed for sagas (§9.1), but that's about command flow between *different* reducers, not where any single reducer's runtime lives.

### 9.4 Frontend command bus scope (slice #3)
Originally planned as single outbound dispatch chokepoint for clicks + slash commands + agent app-API calls. Descoped per conventions Q1+Q4. Open: does a single bus still make sense, or are per-slice dispatchers sufficient? **Blocker for** agent-driven UI mutation audit + policy enforcement.

### 9.5 Saga compensation vs retry on srv crash
PR 1 of saga durability shipped. PR 2 (resume-on-startup) pending. Open: if a saga is mid-flight when srv crashes, how does recovery distinguish "compensate" from "retry"? Today: failed compensation marked for operator review. ([`SPEC_SAGA_DURABILITY_2026-05-01.md`](./SPEC_SAGA_DURABILITY_2026-05-01.md) §9)

### 9.6 E.4.B implementation trigger
Phase 4 helpers shipped. Phases 5–7 designed but not started. Open: what's the trigger? Frontend slice #8 (pane-tree) is the natural consumer, but slice #8 is also blocked on E.4.B. **Recommendation:** ship E.4.B Phase 5 (reducer arms) as soon as Phase 5 of multi-reducer-roadmap claims a CPU cycle, since frontend slice #8 already has a written spec waiting.

### 9.7 Phase G go/no-go itself
Roadmap defers *Phase G* until F.6+F.7+cross-process dispatch. But the decision to *do* Phase G at all is open. Cost/benefit per phase-fg-roadmap §4: Phase E validated the pattern for srv. Still need system-level validation (host reducer + cross-process dispatch) before committing to event-sourced everywhere.

### 9.8 Phase F.7 — host bridge resilience — ✅ LANDED + ROOT CAUSE FOUND DOWNSTREAM (2026-05-07)

**Three-layer defense-in-depth shipped:**
- **Launcher (PR #708, #721):** per-window monotonic `*_emitted: bool` storm caps on `HiddenSinceOpen`, `OffMonitor`, `CorrectiveWindowMove`. PR #722 round 4 extended the `OffMonitor` cap to `apply_monitor_topology_changed` so display hot-plug doesn't bypass it.
- **Host bridge (PR #722):** `launcher_event_bridge::DedupCache` (FIFO-evicting, 4096-key cap) on `(variant_kind, label, hwnd, version)` — same gate the renderer used (§8.14), promoted to host so a cold V8 context post-crash can't replay older versions.
- **Renderer (PR #708):** `shouldDispatchLauncherEvent` per-key dedup in `frontend/util/launcher-events.ts`.

**But the actual root cause was downstream of all three caps.** PR #724 (2026-05-07) found the storm crashes that #708/#721/#722 chased were caused by an unintentional SolidJS reactive-dep leak in `recordDispatch` (`frontend/app/store/command-source.ts`). The function read `recordsAtom()` and wrote `setRecordsAtom(...)` in the same call. The launcher-event reducer's `createEffect` (`launcher-event-reducer.ts:148`) called `dispatch → recordDispatch`; the bare getter inside the reactive context registered `recordsAtom` as a tracked dep, and the subsequent set re-fired the effect → infinite loop.

**Diagnostic recipe** (worked on v0.33.695): instrument `__agentmux_launcher_event` bridge call count + `createEffect` re-run count; if effect-runs ≫ bridge-calls for the same event, the bug is renderer-side reactive, not host-side fan-out. On the failing repro: 1 bridge call → 3230+ effect re-runs → 463 console.warn → V8 stack exhaustion → Crashpad. Post-fix: 1 bridge call → 1 effect run.

**Fix landed in PR #724** (merged 2026-05-07 as 6b84df2a). `recordDispatch` now wraps the read+write in `untrack()`. Regression test: `command-source.test.ts::does not register reactive deps when called from inside createEffect` (asserts effect runs exactly once per explicit trigger; fails on the un-fixed code).

The host/launcher caps from #708/#721/#722 are kept as defense-in-depth — they don't hurt and they cap any future regressions in the launcher's emit cadence — but the actual amplifier was removed in #724.

**Lesson for future renderer storm crashes:** look at SolidJS effect re-run counts BEFORE blaming host fan-out. See [`ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`](./ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md) §4.4 for the per-layer history.

---

## 10. Superseded ledger

These are docs whose status content is now captured here. **They retain value as historical decision records and design rationale.** Don't blindly delete — but if you're cleaning up, you can safely move them to `docs/retro/archive/` or delete after auditing nothing in-flight references them.

| Doc | Why superseded |
|---|---|
| [`multi-reducer-status-2026-04-29.md`](../retro/multi-reducer-status-2026-04-29.md) | Cross-layer status table replaced by §2–§7 here |
| [`phase-e-status-2026-04-30.md`](../retro/phase-e-status-2026-04-30.md) | Pre-E.2c snapshot, already superseded by 2026-05-01 |
| [`phase-e-tear-off-and-remaining-2026-04-30.md`](../retro/phase-e-tear-off-and-remaining-2026-04-30.md) | Folded into phase-e-status §3.3 |
| [`next-steps-2026-04-29.md`](../retro/next-steps-2026-04-29.md) | Earlier ordering snapshot, replaced by 2026-05-02 |
| [`reducer-architecture-current-state-2026-05-02.md`](../retro/reducer-architecture-current-state-2026-05-02.md) | Mid-Phase-E synthesis, replaced by this doc |
| [`PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30.md`](./PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30.md) | Pre-architecture-completeness saga plan, replaced by [`SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md`](./SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md) |
| [`SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md`](./SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md) | Alternate framing of host reducer; canonical version is [`SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`](./SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md). Keep as historical, treat F as source of truth. |
| [`SPEC_LAYOUT_HEAL_ROOTNODE_ORPHAN.md`](./SPEC_LAYOUT_HEAL_ROOTNODE_ORPHAN.md) | Stub (no date, no body of substance). Likely safe to delete after audit. |

**Decision deferred to user:** I haven't deleted or moved any of these. If you want, I can open a follow-up PR moving the `superseded` entries to `docs/retro/archive/` and deleting the `stub` row.

---

## 11. Authoritative source docs (when you need detail)

**Architecture / proposals:**
- [`multi-reducer-proposal-2026-04-28.md`](../retro/multi-reducer-proposal-2026-04-28.md) — original three-reducer vision
- [`b5-migration-architecture-2026-04-28.md`](../retro/b5-migration-architecture-2026-04-28.md) — why `browsers` can't migrate
- [`migration-pattern.md`](../retro/migration-pattern.md) — the a→b→c→d→e ratchet

**Per-layer specs:**
- Launcher: implicit in [`phase-b-roadmap.md`](../retro/phase-b-roadmap.md) decisions log
- Host: [`SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`](./SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md) + [`SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md`](./SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md)
- Srv: [`SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md`](./SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md) + [`SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md`](./SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md) + [`srv-phase-e4b-formal-spec-2026-05-03.md`](./srv-phase-e4b-formal-spec-2026-05-03.md)
- Frontend: [`frontend-reducer-architecture-2026-05-03.md`](./frontend-reducer-architecture-2026-05-03.md) + [`frontend-reducer-conventions-2026-05-03.md`](./frontend-reducer-conventions-2026-05-03.md) + [`frontend-reducer-implementation-plan-2026-05-03.md`](./frontend-reducer-implementation-plan-2026-05-03.md)

**Sagas:**
- [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) — canonical vision
- [`SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md`](./SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md) — PR-by-PR roadmap
- [`SPEC_PHASE_E_SAGAS_2026-04-30.md`](./SPEC_PHASE_E_SAGAS_2026-04-30.md) — E.5 design
- [`SPEC_SAGA_DURABILITY_2026-05-01.md`](./SPEC_SAGA_DURABILITY_2026-05-01.md) — durability + recovery
- [`saga-coordinator-location-analysis-2026-04-30.md`](../retro/saga-coordinator-location-analysis-2026-04-30.md) — why srv-side won

**Status retros (still authoritative; this doc collates rather than replaces):**
- [`phase-b-roadmap.md`](../retro/phase-b-roadmap.md) — Phase B
- [`phase-e-status-2026-05-01.md`](../retro/phase-e-status-2026-05-01.md) — Phase E
- [`phase-fg-status-2026-05-01.md`](../retro/phase-fg-status-2026-05-01.md) — Phase F+G shipped/deferred
- [`phase-fg-roadmap-2026-05-01.md`](../retro/phase-fg-roadmap-2026-05-01.md) — Phase F+G remaining

---

## 12. Recently shipped (2026-05-04 → 2026-05-05)

| PR | Title | Layer |
|---|---|---|
| [#688](https://github.com/agentmuxai/agentmux/pull/688) | typed LayoutNode struct (E.4.B Phase 2) + tabN naming fix | srv |
| [#690](https://github.com/agentmuxai/agentmux/pull/690) | layout Command/Event wire types (E.4.B Phase 3) | srv |
| [#691](https://github.com/agentmuxai/agentmux/pull/691) | E.4.B Phase 4 — pure layout tree helpers | srv |
| [#692](https://github.com/agentmuxai/agentmux/pull/692) | insert_node_at_index TS-oracle clamp follow-up | srv |

40 layout tests now passing locally. E.4.B Phase 5 (reducer arms calling these helpers) is unblocked.

---

*Master spec written 2026-05-05. Update §2 status table and §5/§6 phase tables as PRs merge; everything else should remain stable until a major architectural shift.*
