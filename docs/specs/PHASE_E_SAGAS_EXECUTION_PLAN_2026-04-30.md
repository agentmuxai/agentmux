# Phase E.5 Sagas — Execution Plan

**Date:** 2026-04-30
**Companion to:** `docs/specs/SPEC_PHASE_E_SAGAS_2026-04-30.md` (the design spec)
**Goal:** finish migrating every wcore-direct mutation through the srv reducer via saga or atomic command, removing the inconsistency window that produced the tear-off → CreateTab regression.

---

## 0. Decisions made

- **Skip the Option-A hot-fix.** The robust solution (sagas) is what we're shipping. Smoke regression on tear-off → "+" tab persists until **PR 3** lands.
- **No partial migration.** Every wcore-direct path in `phase-e-tear-off-and-remaining-2026-04-30.md` §3 either becomes an atomic reducer command (single-step) or a saga (multi-step). End state: reducer is sole writer of workspace/tab/block/window/layout state.
- **PRs bundle aggressively** to match the established cadence (the E.2c series merged 7 PRs in one session).

---

## 1. PR sequence (4 PRs)

| PR | Branch | Scope | Est LoC | Smoke regression fixed? |
|---|---|---|---|---|
| **PR 1: E.5.1+2** | `agenta/phase-e-5-foundation` | Saga trait + correlation IDs + window state in reducer | ~650 | No (foundation only) |
| **PR 2: E.5.3+4** | `agenta/phase-e-5-atomic-commands` | New atomic commands + reducer arms + subscriber applies + RPC migration of all 7 single-step paths | ~900 | No (low-risk paths only) |
| **PR 3: E.5.5+6** | `agenta/phase-e-5-tear-off-sagas` | TearOffTab + TearOffBlock + RestoreTornOffTab sagas | ~800 | **YES — tear-off → "+" tab works** |
| **PR 4: E.5.7+8+9** | `agenta/phase-e-5-window-sagas-and-cleanup` | MoveTabToWorkspace + MoveBlockToTab + CreateWindow + CloseWindow sagas + helper retirement | ~750 | All wcore-direct migrations done |

**Total: ~3100 LoC across 4 PRs.** Each individually shippable + smoke-testable.

### Parallel work (not on the saga critical path)

- **E.2c.5b** (TypeScript renderer dispatcher) — independent of saga work; can ship at any point. ~200-300 LoC frontend.
- **E.4** (Layout minimal slice) — independent of saga work. ~250 LoC.
- **E.6** (renderer per-source version + saga buffer) — depends on E.2c.5b and on saga lifecycle events.
- **E.7** (proptests + integration tests + `--diag`) — phase exit; depends on all of above.

---

## 2. PR 1 — Foundation (E.5.1 + E.5.2)

### Branch
`agenta/phase-e-5-foundation`

### Scope

**E.5.1: Saga trait + correlation IDs + coordinator wiring**

- Extend the `Saga` trait in `agentmux-launcher/src/saga.rs`:
  - Add `fn compensate(&mut self) -> Vec<Command>` (default impl: empty Vec).
  - Confirm/extend `on_event(&mut self, event: &Event) -> SagaStep` matches the spec §4.1 signature.
  - Add `SagaStep::Continue / Completed / Failed` variants.
- Add optional `saga_id: Option<u64>` to **every** `Command` and `Event` variant in `agentmux-common/src/ipc.rs`. `#[serde(default)]` on each so old log entries remain compatible.
- Reducer (both launcher and srv) copies `saga_id` from incoming Command into emitted Events.
- Coordinator changes:
  - `register(saga: Box<dyn Saga>, completion_tx: oneshot::Sender<SagaOutcome>)` — RPC handlers register and await completion.
  - On `SagaStep::Failed`, dispatch `compensate()` commands in reverse order; emit `SagaFailed`; resolve oneshot with `SagaOutcome::Failed(reason)`.
  - On `SagaStep::Completed { result }`, emit `SagaCompleted`; resolve oneshot with `SagaOutcome::Completed(result)`.
- Add `dispatch_saga<S: Saga + 'static>(state: &AppState, saga: S) -> Result<serde_json::Value, String>` helper in service.rs that wraps register + await + timeout (5s default).

**E.5.2: Window state in reducer + bootstrap**

- New `WindowRecord { window_id: String, workspace_id: String }` in `agentmux-srv/src/state.rs`.
- New `windows: HashMap<String, WindowRecord>` field in State.
- New `Command::CreateWindow { window_id, workspace_id }` and `Command::CloseWindowInternal { window_id }` and `Command::SwitchWorkspace { window_id, workspace_id }` (all atomic, dispatched by sagas in later PRs).
  - **Note:** these are introduced here so PR 1 has full window-state plumbing without behavior change. PR 2 wires them into RPC handlers; PR 3-4 wire them into sagas.
- New `Event::WindowOpened`, `Event::WindowClosed`, `Event::WindowWorkspaceChanged` (srv-side; the launcher already has its own WindowOpened — these are scoped to srv state).
  - **Naming clash with launcher's `WindowOpened`:** rename srv's variants to `Event::SrvWindowOpened` etc. to avoid ambiguity.
- Reducer arms for the three commands (validation, mutation, event emission).
- Bootstrap (`persist::bootstrap_state_from_wstore`) loads windows from wstore alongside workspaces/tabs/blocks.
- Persist subscriber gains `apply_srv_window_opened`, `apply_srv_window_closed`, `apply_srv_window_workspace_changed`.
- `extract_version` updated in srv reducer + launcher reducer + both event_log files for the three new events.
- Misrouted-srv arms in launcher reducer + ipc/server for the three new commands.

### What stays the same in PR 1
- No RPC handlers change behavior. `("window", "CreateWindow")` etc. still call `wcore::*` directly.
- No sagas implemented yet — just the framework.
- No tear-off behavior change.

### Tests
- Reducer tests for the three window commands: happy path + validation errors.
- Saga trait property tests: `compensate` invoked on failure; correlation IDs propagate.
- Bootstrap test: existing wstore Window rows load into `state.windows`.

### Pre-merge smoke
None practical — no behavior change. CI green is the only gate.

---

## 3. PR 2 — Atomic commands + RPC migration (E.5.3 + E.5.4)

### Branch
`agenta/phase-e-5-atomic-commands`

### Scope

**E.5.3: New atomic commands + reducer arms + subscriber applies**

New commands (all atomic, no saga):
```rust
Command::PromoteBlockToTab { block_id, src_tab_id, workspace_id }
Command::ReorderTabsBulk { workspace_id, tab_ids: Vec<String> }
Command::RenameWorkspace { workspace_id, name }
Command::RenameTab { tab_id, name }
Command::UpdateBlockMeta { block_id, meta_patch: serde_json::Value }
Command::UpdateWorkspaceMeta { workspace_id, meta_patch }
Command::UpdateTabMeta { tab_id, meta_patch }
Command::MoveTab { tab_id, src_workspace_id, dst_workspace_id, dst_index }
Command::MoveBlock { block_id, src_tab_id, dst_tab_id, dst_index }
Command::DeleteWorkspaceCascade { workspace_id }  // alias for DeleteWorkspace; explicit naming for sagas
```

Corresponding events:
```rust
Event::BlockPromotedToTab { block_id, new_tab_id, workspace_id, version }
Event::TabsReorderedBulk { workspace_id, tab_ids, version }
Event::WorkspaceRenamed { workspace_id, name, version }
Event::TabRenamed { tab_id, name, version }
Event::BlockMetaUpdated { block_id, meta, version }   // emits the resolved meta map, not the patch
Event::WorkspaceMetaUpdated { workspace_id, meta, version }
Event::TabMetaUpdated { tab_id, meta, version }
Event::TabMoved { tab_id, src_workspace_id, dst_workspace_id, dst_index, version }
Event::BlockMoved { block_id, src_tab_id, dst_tab_id, dst_index, version }
```

**Reducer state additions:** add `meta` field to `WorkspaceRecord`, `TabRecord`, `BlockRecord`. Bootstrap populates from wstore. All test usages of these structs need the new field.

Reducer arm impl for each new command (validation + mutation + event emission). Tests for each.

Subscriber apply functions for each new event. Tests for each.

`extract_version` updated everywhere (srv reducer + event_log + launcher reducer + launcher event_log).

Misrouted-srv arms in launcher for each new command.

**E.5.4: RPC migration of single-step paths**

In `agentmux-srv/src/server/service.rs`, migrate these handlers from wcore-direct to reducer-dispatch:

| RPC handler | Old path | New path |
|---|---|---|
| `("workspace", "PromoteBlockToTab")` | `wcore::promote_block_to_tab` | dispatch `Command::PromoteBlockToTab` |
| `("workspace", "UpdateTabIds")` | direct `store.update::<Workspace>` | dispatch `Command::ReorderTabsBulk` |
| `("workspace", "UpdateWorkspace")` | direct `store.update::<Workspace>` | dispatch `Command::RenameWorkspace` (and/or `UpdateWorkspaceMeta` if meta-only) |
| `("object", "UpdateTabName")` | direct `store.update::<Tab>` | dispatch `Command::RenameTab` |
| `("object", "UpdateObjectMeta")` | dispatch helper that decomposes by otype | dispatch `Command::UpdateBlockMeta` / `UpdateTabMeta` / `UpdateWorkspaceMeta` |
| `("object", "UpdateObject")` | generic deserialize + update | decompose at RPC layer to typed Update*Meta or Rename* commands |
| `("window", "SwitchWorkspace")` | direct `store.update::<Window>` | dispatch `Command::SwitchWorkspace` |

Each follows the now-established forward+compensate pattern (or SQLite-first for inverse-difficult paths). All migrated handlers exit forward+compensate territory because they only mutate existing entities, not create new ones — no compensation needed, just dispatch + apply + publish.

### What stays the same in PR 2
- Tear-off, restore, multi-step move paths still wcore-direct (saga-shape; saved for PR 3-4).
- CreateWindow / CloseWindow still wcore-direct (saga-shape; PR 4).

### Tests
- Reducer tests for each new arm (10+ tests).
- Subscriber tests for each new apply (10+ tests).
- RPC handler smoke (manual): rename a workspace, rename a tab, reorder tabs (drag), edit tab meta — verify SQLite + reducer state in sync.

### Pre-merge smoke
- Rename workspace, rename tab → persists across restart.
- Reorder tabs (drag) → order persists.
- Block meta edits via UI → still works.
- **No expected fixes for tear-off.**

---

## 4. PR 3 — Tear-off + Restore sagas (E.5.5 + E.5.6) **← FIXES SMOKE REGRESSION**

### Branch
`agenta/phase-e-5-tear-off-sagas`

### Scope

**Three saga implementations** in `agentmux-srv/src/sagas/` (new module):

#### TearOffTab saga
Per spec §6.1:
1. `CreateWorkspace { name: <derived> }` → wait for `WorkspaceCreated`
2. `MoveTab { tab_id, src_ws_id, new_ws_id, dst_index: 0 }` → wait for `TabMoved`
3. `CreateWindow { window_id: <new uuid>, workspace_id: new_ws_id }` → wait for `SrvWindowOpened`
4. `Completed { result: { new_workspace_id, new_window_id } }`

Compensation: reverse `MoveTab` + `DeleteWorkspaceCascade` per step.

#### TearOffBlock saga
Per spec §6.2:
1. `CreateWorkspace` → wait for `WorkspaceCreated`
2. `CreateTab` → wait for `TabCreated`
3. `MoveBlock` → wait for `BlockMoved`
4. `CreateWindow` → wait for `SrvWindowOpened`
5. `Completed { result: { new_workspace_id, new_tab_id, new_window_id } }`

#### RestoreTornOffTab saga
Per spec §6.3:
1. `MoveTab { tab_id, src: torn_ws_id, dst: src_ws_id }` → wait for `TabMoved`
2. Inspect post-move state: if `torn_ws_id` empty → step 3; else → `Completed`
3. `DeleteWorkspaceCascade { torn_ws_id }` → wait for `WorkspaceDeleted`
4. `Completed { result: {} }`

### RPC handler migration

| RPC handler | New impl |
|---|---|
| `("workspace", "TearOffTab")` | `dispatch_saga(state, TearOffTabSaga::new(...))` then return saga's result |
| `("workspace", "TearOffBlock")` | `dispatch_saga(state, TearOffBlockSaga::new(...))` |
| `("workspace", "RestoreTornOffTab")` | `dispatch_saga(state, RestoreTornOffTabSaga::new(...))` |

### What stays the same in PR 3
- `MoveTabToWorkspace`, `MoveBlockToTab`, `CreateWindow`, `CloseWindow` still wcore-direct (PR 4).

### Tests
- Saga property tests per spec §8.2:
  - Happy path for each saga
  - Step-N failure with correct compensation
  - Out-of-order events ignored
- Integration test: real coordinator + reducer + subscriber + in-memory wstore.
- Reducer correctly emits `TabMoved` with all fields populated.

### Pre-merge smoke (CRITICAL)

The smoke regression from `agentmux-0.33.520` should be FIXED:
1. Tear off a tab → new window opens with the tab.
2. **In the new window, click `+` → new tab appears.** ← this is the regression
3. Switch between tabs in the new window — works.
4. Close one of the tabs in the new window — works.
5. Drag-reorder tabs in the new window — works.
6. Restore the torn-off tab (drag back, or "Restore" UI if exists) — tab returns to original window; torn-off window closes; torn-off workspace deleted.

After this PR ships, the user can resume normal smoke testing of the entire app.

---

## 5. PR 4 — Window sagas + cleanup (E.5.7 + E.5.8 + E.5.9)

### Branch
`agenta/phase-e-5-window-sagas-and-cleanup`

### Scope

**E.5.7: MoveTab + MoveBlock cross-workspace sagas**

These are single-step sagas (per spec §6.4 / §6.5). Could be plain reducer commands but using saga shape preserves correlation IDs for renderer buffering (E.6).

```rust
MoveTabSaga { tab_id, src_ws_id, dst_ws_id, insert_index }
MoveBlockSaga { block_id, src_tab_id, dst_tab_id, dst_index }
```

RPC handlers migrated:
- `("workspace", "MoveTabToWorkspace")` → `dispatch_saga(MoveTabSaga::new(...))`
- `("workspace", "MoveBlockToTab")` → `dispatch_saga(MoveBlockSaga::new(...))`

**E.5.8: CreateWindow + CloseWindow sagas**

Per spec §6.6 / §6.7:
- `CreateWindowSaga`: CreateWorkspace → CreateWindow → Completed
- `CloseWindowSaga`: CloseWindowInternal → conditional DeleteWorkspaceCascade → Completed

RPC handlers migrated:
- `("window", "CreateWindow")` → `dispatch_saga(CreateWindowSaga::new(...))`
- `("window", "CloseWindow")` → `dispatch_saga(CloseWindowSaga::new(...))`

**E.5.9: Cleanup**

- Audit `agentmux-srv/src/server/service.rs` for any remaining wcore-direct mutations of reducer-tracked state. Should be zero.
- Remove the `ensure_workspace_in_reducer` helper (if Option-A hot-fix was ever shipped). **Per current plan: never shipped, so nothing to remove.**
- Drop `wstore_workspace_exists` if no longer used (was added for DeleteWorkspace fallback in E.2c.2 / E.2c.3; now redundant).
- Update `docs/retro/multi-reducer-status-2026-04-29.md` and `phase-e-status-2026-04-30.md` to mark E.5 complete.

### What stays the same in PR 4
- Phase E.6 + E.7 still ahead — renderer multi-source + saga buffering, then phase exit.

### Tests
- Saga tests for MoveTab, MoveBlock, CreateWindow, CloseWindow.
- Property test: after E.5.9, no `service.rs` handler calls `wcore::create_*` / `wcore::delete_*` / `wcore::move_*` directly. (Could be a clippy lint or a grep-based CI check.)

### Pre-merge smoke
- Move tab between workspaces (drag) — works, persists.
- Move block between tabs in different workspaces — works, persists.
- Create new window via UI → workspace + window land in SQLite + reducer.
- Close window → workspace deleted if last reference.
- Full app exercise — every existing user-visible flow should work end-to-end.

---

## 6. Per-PR completion criteria

Each PR ships when:

1. All listed scope items implemented.
2. All listed tests pass.
3. `cargo build --release -p agentmux-srv -p agentmux-cef -p agentmux-launcher` clean.
4. `cargo test -p agentmux-srv --release` passes.
5. Reagent + Codex APPROVE the PR (or non-blocking only).
6. Pre-merge smoke (where defined) verified manually on a portable build.
7. Retro doc updated.

---

## 7. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| Tear-off remains broken in production through PRs 1-2 (3-5 days?) | Medium | User aware; smoke for non-tear-off paths still works. |
| Saga timeout (5s default) too aggressive | Low | Tunable; raise to 30s if any saga shows latency in proptests. |
| Adding meta to WorkspaceRecord/TabRecord/BlockRecord breaks ~30 existing tests | Medium | Mechanical fix (similar to Block meta in E.2c.4); budget half a session. |
| New events break the launcher's `extract_version` exhaustive match | Low | Caught at compile time; mechanical fix per new variant. |
| Compensation logic incorrect → state diverges on saga failure | High | Property tests cover every step's compensation; smoke each saga's failure path. |
| `dispatch_saga` oneshot future leaks if the saga never terminates | Medium | Timeout wraps the await; coordinator force-fails the saga on timeout. |
| Saga IDs collide across processes (counter not unique cross-instance) | Low | Each instance has its own counter; not shared; saga_id is per-instance scope. Cross-process correlation isn't a requirement until Phase F. |

---

## 8. Carry-forwards from prior PRs

These are non-blocking but should fold into one of the PRs above when convenient:

- Codex P2 from #613 — ambiguous block-parent during bootstrap. Fold into PR 2's bootstrap changes (PR 2 already touches bootstrap to load meta).
- Whatever new codex notes appear on #618 post-merge — fold into PR 1 if compatible.

---

## 9. Out of scope for E.5

- **TypeScript renderer dispatcher** (E.2c.5b) — separate frontend PR.
- **Renderer per-source version tracking + saga buffer** (E.6) — depends on E.2c.5b + saga events.
- **Property tests for full Phase E** (E.7) — phase exit; covers all of D, E, F.
- **Layout state migration** (E.4) — separate; tracks focused/magnified node only in the minimal slice.
- **Cross-process saga correlation** — Phase F or beyond.

---

## 9b. Post-E.5 follow-up PRs (small, scheduled)

Two cheap correctness gaps worth filing now so they don't get lost. Both are documented in `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` §6.3-§6.4.

| ID | Scope | LoC | When | Why |
|---|---|---|---|---|
| **F1.A** | Wrap each `apply_*_event` arm in the persist subscriber in a `wstore` BEGIN/COMMIT transaction. Today's subscriber does sequential `store.update`/`store.insert` calls non-transactionally; partial failures can leave half-written rows. | ~200 | Inside PR 4 (low extra LOC) or standalone | Cheapest robustness win. Real correctness gap that exists today, not a Phase F problem. |
| **F1.B** | Frontend cleanup on host pool-promote failure: in `tabbar.tsx::requestTearOff` (and block-tear-off equivalents), if `tear_off_pool_promote` returns `pool_exhausted` AND `open_window_at_position` then fails, dispatch `WorkspaceService.DeleteWorkspaceCascade` to clean up the orphan workspace just created via `TearOffTab`. Show a user-facing error toast. | ~50 TS | Standalone, post-E.5 | Trivial fix for a real failure mode that produces visible junk in the workspace list. |

The other three robustness gaps (host as saga step, renderer registration as saga step, persistent saga state) are deferred until Phase F has a written spec. They are not currently scoped or scheduled.

---

## 10. Decision log

- **2026-04-30:** plan written. 4-PR structure agreed. Hot-fix Option A skipped in favor of going straight to sagas.
- **2026-04-30:** coordinator location decided as srv-side (Path A). See `docs/retro/saga-coordinator-location-analysis-2026-04-30.md`.
- **2026-04-30:** post-E.5 follow-ups F1.A (subscriber transactions) and F1.B (frontend orphan cleanup) added to plan §9b.
- Refer to `SPEC_PHASE_E_SAGAS_2026-04-30.md` for the design spec; this plan is the execution sequence.
