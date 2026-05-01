# Phase E Multi-Reducer Migration — Consolidated Status (2026-05-01)

**Date:** 2026-05-01
**Branch state:** `main` at `7714e06b` (post-F1.B merge)
**Phase E PRs open:** none — loop stopped after F1.B merged.
**Current version:** 0.33.532

This doc consolidates and supersedes:
- `docs/retro/phase-e-status-2026-04-30.md` (pre-E.2c, now stale)
- `docs/retro/phase-e-tear-off-and-remaining-2026-04-30.md` (analysis that motivated E.5)
- Sub-phase rows in `docs/retro/multi-reducer-status-2026-04-29.md`

It's the single authoritative read for "where Phase E stands and what's next."

---

## 1. TL;DR

- **Phase E.5 is complete.** The smoke regression (tear-off → "+" tab → nothing happens) is fixed. Every workspace/tab/block/window state mutation in `agentmux-srv` routes through the srv reducer.
- **Two robustness follow-ups also shipped** (F1.A subscriber transactions, F1.B frontend orphan-cleanup).
- **Phase E remaining sub-phases:** E.2c.5b (TS renderer dispatcher), E.4 (layout state migration), E.6 (renderer multi-source + saga buffer), E.7 (property tests + diag tools). All are designed-but-not-implemented.
- **Phase F+ deferrals:** cross-process saga (host pool-promote inside the saga), persistent saga state, host reducer migration. No spec written yet.
- **Smoke status: not yet performed.** All test coverage is unit-level. Recommended: smoke now (see §8).

---

## 2. Phase E sub-phase table

| Sub-phase | PR | Scope | Status |
|---|---|---|---|
| **E.1a** | [#609](https://github.com/agentmuxai/agentmux/pull/609) | Saga coordinator framework (launcher) — left as labeled stub since E.5 chose srv-side coordinator. See §6. | ✅ |
| **E.1b** | [#610](https://github.com/agentmuxai/agentmux/pull/610) | Srv reducer skeleton + new srv pipe + broadcast bus + event log | ✅ |
| **E.2** | [#611](https://github.com/agentmuxai/agentmux/pull/611) | Workspace lifecycle arms | ✅ |
| **E.2b** | [#612](https://github.com/agentmuxai/agentmux/pull/612) | Tab + ActiveTab arms | ✅ |
| **E.3** | [#613](https://github.com/agentmuxai/agentmux/pull/613) | Block lifecycle arms | ✅ |
| **E.2c.1** | [#614](https://github.com/agentmuxai/agentmux/pull/614) | Persist subscriber plumbing (idempotent SQLite write-back) | ✅ |
| **E.2c.2** | [#615](https://github.com/agentmuxai/agentmux/pull/615) | Workspace RPC migration | ✅ |
| **E.2c.3 / E.2c.3b** | [#616](https://github.com/agentmuxai/agentmux/pull/616), [#617](https://github.com/agentmuxai/agentmux/pull/617) | Tab RPC migration (CreateTab + SetActiveTab + CloseTab + ReorderTab); pinning removed | ✅ |
| **E.2c.4 + E.2c.5a** | [#618](https://github.com/agentmuxai/agentmux/pull/618) | Block RPC migration + Rust host bridge to srv pipe | ✅ |
| **E.2c.5b** | — | TypeScript renderer dispatcher (`window.__agentmux_srv_event` + atom routing) | ⛔ not implemented |
| **E.5.1+2** | [#619](https://github.com/agentmuxai/agentmux/pull/619) | Saga foundation (window state + design docs) | ✅ |
| **E.5.3+4** | [#620](https://github.com/agentmuxai/agentmux/pull/620) | Atomic single-step commands + RPC migration | ✅ |
| **E.5.5+6** | [#621](https://github.com/agentmuxai/agentmux/pull/621) | **Tear-off + restore sagas — fixes the smoke regression** | ✅ |
| **E.5.7+8+9** | [#622](https://github.com/agentmuxai/agentmux/pull/622) | Final wcore-direct migrations + cleanup | ✅ |
| **F1.A** | [#623](https://github.com/agentmuxai/agentmux/pull/623) | Subscriber SQLite transactions | ✅ |
| **F1.B** | [#624](https://github.com/agentmuxai/agentmux/pull/624) | Frontend orphan-cleanup on tear-off cold-path failure | ✅ |
| **E.4** | — | Layout state arms (focused/magnified node + pendingbackendactions) | ⛔ not implemented |
| **E.6** | — | Renderer multi-source + saga buffering | ⛔ not implemented |
| **E.7** | — | Property tests + integration tests + `--diag srv` / `--diag sagas` | ⛔ not implemented |

---

## 3. What was actually shipped this session (2026-04-30 → 2026-05-01)

**6 PRs, ~6700 LOC, fixed the user-visible smoke regression and closed two robustness gaps.**

### 3.1 Saga foundation (#619)
- Added `Window` state to srv reducer (`WindowRecord` + `state.windows: HashMap<window_id, WindowRecord>`).
- New commands: `CreateWindow`, `CloseWindowInternal`, `SwitchWorkspace`. New events: `SrvWindowOpened`, `SrvWindowClosed`, `SrvWindowWorkspaceChanged`.
- Bootstrap loads window rows; `DeleteWorkspace` cascades to drop window mappings; `Client.windowids` stays in sync via subscriber.
- Wrote three design docs: `SPEC_PHASE_E_SAGAS_2026-04-30.md`, `PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30.md`, `phase-e-tear-off-and-remaining-2026-04-30.md`.

### 3.2 Atomic single-step commands (#620)
- 6 new commands: `ReorderTabsBulk`, `RenameWorkspace`, `RenameTab`, `Update{Workspace,Tab,Block}Meta`. 6 new events.
- 4 RPC handlers migrated: `UpdateWorkspace` → `RenameWorkspace`, `UpdateTabIds` → `ReorderTabsBulk`, `UpdateTabName` → `RenameTab`, `UpdateObjectMeta` → decompose by otype.
- New `merge_meta_patch` helper preserves `merge_meta`'s `section:*` clear semantics.
- `apply_tabs_reordered_bulk` drains legacy `Workspace.pinnedtabids` to prevent UI double-counting (pinning was removed in E.2c.3b but legacy SQLite rows linger).

### 3.3 Tear-off sagas — smoke regression fix (#621)
- New `agentmux-srv/src/sagas/` module: coordinator + 3 sagas.
  - `TearOffTabSaga` — CreateWorkspace + MoveTab.
  - `TearOffBlockSaga` — CreateWorkspace + CreateTab + MoveBlock.
  - `RestoreTornOffTabSaga` — MoveTab back + conditional DeleteWorkspaceCascade.
- `dispatch` / `compensate` / `state_lock` helpers, 5 s timeout wrapper.
- New wire types: `Command::MoveTab`, `Command::MoveBlock`, `Event::TabMoved` (with `new_src_active_tab_id` + `new_dst_active_tab_id`), `Event::BlockMoved`.
- Reducer's `handle_move_tab` made migration-tolerant (lazy-imports unknown tabs, drops workspace_id check) since wcore-direct paths could leave reducer state stale.
- `MoveTabToWorkspace` migrated through the reducer to keep `state.tabs` in sync (was the codex P1 trigger that revealed the staleness gap).
- `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` — coordinator-location decision (Path A: srv) + full robustness audit + gap analysis.

### 3.4 Final wcore migrations (#622)
- `MoveBlockToTab` → `Command::MoveBlock` + auto-close via `Command::DeleteTab`.
- `PromoteBlockToTab` → new saga (CreateTab + MoveBlock); layout setup / SetActiveTab / auto-close stay wcore-direct in handler (E.4 layout territory).
- `CreateWindow` → multi-step inline reducer dispatch (CreateWorkspace + CreateTab + CreateWindow when empty workspace_id).
- `CloseWindow` → CloseWindowInternal + conditional DeleteWorkspaceCascade.
- `SwitchWorkspace` → single-step `Command::SwitchWorkspace` dispatch.
- `handle_create_tab` auto-generates `tabN` when name is empty (mirrors wcore behaviour, fixes blank-title regression).
- Cleanup audit confirms only SQLite-first delete patterns remain wcore-direct.

### 3.5 Subscriber SQLite transactions (#623)
- 6 multi-write apply arms wrapped in `wstore.with_tx(|tx| ...)`:
  - `apply_tab_created`, `apply_block_created`, `apply_srv_window_opened`, `apply_srv_window_closed`, `apply_tab_moved`, `apply_block_moved`.
- Closes the per-step atomicity gap from §4.2 of the saga analysis. A partial failure now rolls back instead of leaving half-applied state on disk.

### 3.6 Frontend orphan-cleanup (#624)
- In `tabbar.tsx::requestTearOff`, if `openWindowAtPosition` itself throws (cold-path API rejection — host couldn't post the create command), dispatch `RestoreTornOffTab` to put the tab back. Saga also cascade-deletes the empty new workspace.
- Provably-safe single signal: anything else (handshake errors, timeouts, post-create failures) leaves the orphan workspace for user cleanup. The 4-round review cycle on this PR (codex flagged 3 successive variants) yielded the strongest version: the only restore signal is a synchronous host rejection.

---

## 4. End-state architecture

After this session, `agentmux-srv` has:

```
HTTP/WS RPC handlers (server/service.rs)
        │
        ├── single-step commands → dispatch_to_reducer → reducer arms
        │                          ↓
        │                          srv reducer (state.rs)
        │                          ↓
        │                          srv broadcast bus
        │                          ↓
        │                          ├── persist_subscriber → SQLite (wstore)
        │                          ├── srv_event_bridge → CEF host JS bridge → renderer
        │                          └── (future) saga coordinator subscribers
        │
        └── multi-step ops → sagas/* (Path A: srv-side coordinator)
                              ├── TearOffTabSaga
                              ├── TearOffBlockSaga
                              ├── RestoreTornOffTabSaga
                              └── PromoteBlockToTabSaga
                                  ↓
                                  dispatches reducer commands sequentially
                                  with saga_id-tagged lifecycle events
                                  (SagaStarted / SagaCompleted / SagaFailed)
```

**Three remaining wcore-direct paths in `service.rs`** — all SQLite-first delete patterns that predate this session and are intentional:
- `wcore::delete_block` (in DeleteBlock handler)
- `wcore::delete_workspace` (in DeleteWorkspace handler)
- `wcore::delete_tab` (in CloseTab handler)

These mutate SQLite first, then dispatch the reducer command (silent on missing). Acceptable trade-off; tightening them is a Phase F+ cleanup if it surfaces as a problem.

**Saga coordinator location:** srv-side (Path A). The launcher's E.1a coordinator framework is a labeled stub. Full reasoning: `docs/retro/saga-coordinator-location-analysis-2026-04-30.md`.

---

## 5. Robustness — what's guaranteed and what isn't

Cross-references the gap audit at `saga-coordinator-location-analysis-2026-04-30.md` §4-§5.

| Concern | Status |
|---|---|
| Reducer state ↔ SQLite consistency at saga step boundaries | ✅ subscriber's idempotent apply |
| Saga compensation on srv-side step failure | ✅ explicit per-saga unwind |
| Per-step SQLite atomicity (multi-row writes inside a single apply) | ✅ wrapped in `with_tx` (F1.A) |
| Frontend orphan-cleanup on hard host failure (cold-path API throws) | ✅ F1.B |
| Smoke regression (tear-off → "+" tab) | ✅ E.5.5+6 |
| Saga_id correlation infrastructure (unblocks E.6 buffering) | ✅ lifecycle events emitted |
| Cross-process atomicity — host pool-promote inside saga | ❌ Phase F+ |
| Renderer-side `(window_id, workspace_id)` registration as saga step | ❌ Phase F+ |
| Saga state durability across srv crash | ❌ Phase F+ |
| Frontend orphan-cleanup on soft host failure (API returned label, window never registered) | ❌ deliberate trade-off — codex P1 round-3 review on F1.B led to skipping this case to prevent the worse "delete workspace, dangling window registers later" outcome |

---

## 6. Key decisions made this session (don't relitigate)

1. **Saga coordinator goes in srv, not launcher.** Path A — every saga in the E.5 plan mutates only srv state. The launcher's E.1a coordinator stays as a labeled stub for hypothetical future cross-process sagas. Reasoning + alternatives in `saga-coordinator-location-analysis-2026-04-30.md`.

2. **Saga shape is async functions, not trait objects with closures.** Avoided HRTB lifetime headaches with SagaCtx<'a> by having `run_saga(name, fut)` take a future directly + helper functions for lifecycle events. Trade-off: each saga has a small lifecycle-management preamble (~6 lines) instead of a closure wrapping. Reads cleanly.

3. **Migration-tolerant reducer for `MoveTab`.** Rather than block all wcore-direct paths simultaneously, `handle_move_tab` lazy-imports unknown tabs and drops the workspace_id check. Documented as an explicit migration-window decision; PR 4's audit confirmed all major wcore-direct creators were migrated, so the tolerance is now mostly defensive rather than load-bearing.

4. **Phase E.5 is a partial robustness improvement.** The honest scope of what E.5 fixes vs what stays open is documented in `saga-coordinator-location-analysis-2026-04-30.md` §6.7-§6.8 and the saga spec §13. Anyone reading this status doc and inferring "tear-off is now fully transactional" is mistaken — it's srv-side-atomic; cross-process orphans remain possible.

5. **F1.B's restore-on-cold-path-throw is the only provably-safe signal.** After 3 codex review rounds, the simplest correct behaviour: only restore when `openWindowAtPosition` itself throws (host couldn't post the create command). All other "did the window open?" heuristics (label-returned-eagerly, handshake error string match, HWND timeout) were either over-eager (orphan window dangling) or relied on brittle assumptions. Other failure paths leave the orphan workspace for user cleanup.

---

## 7. What's open

### 7.1 Phase E sub-phases not yet implemented

#### E.2c.5b — TypeScript renderer dispatcher
- Install `window.__agentmux_srv_event` handler (the host bridge already forwards events; renderer just needs to route them into atom domains).
- Status: not started. Spec is in `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §6.6.
- Estimated effort: ~150 LOC TS.

#### E.4 — Layout state arms
- Reducer commands for `LayoutState` mutations: focused/magnified node, pendingbackendactions, leaforder.
- Open design question: **granularity.** A "set rootnode" command is too coarse (every drag-resize fires); a "patch node N's size" command needs a node-path representation. The minimal slice (focused/magnified only) is straightforward; the full layout migration is a phase in itself.
- Status: not started. Several callers in `wcore::tear_off_block` / `wcore::promote_block_to_tab` still write to layout via wcore.
- Estimated effort: ~500-1000 LOC depending on granularity decision.

#### E.6 — Renderer multi-source consumption + saga buffering
- Renderer currently consumes a single event source. Needs:
  - Per-source version tracking (`launcher_event_version` vs `srv_event_version` so dropped events are detectable).
  - Saga buffering: when `SagaStarted { saga_id }` arrives, buffer subsequent saga_id-tagged events; flush atomically on `SagaCompleted`.
  - Resync ordering: snapshot replay must precede live-event application.
- Depends on: E.2c.5b (renderer dispatcher).
- Status: not started. Saga_id correlation infra is in place from E.5.
- Estimated effort: ~400 LOC TS.

#### E.7 — Property tests + integration tests + diag tools
- Property tests: reducer arm invariants (cascade integrity, idempotency, version monotonicity).
- Integration tests: real coordinator + reducer + subscriber + in-memory wstore. End-to-end saga happy paths + crash-mid-saga recovery.
- Diag tools: `--diag srv` (state-now), `--diag sagas` (in-flight registry). Mirror the launcher's `--diag wrr` pattern.
- Status: not started. Phase E exit criterion.
- Estimated effort: ~600 LOC.

### 7.2 Phase F+ deferrals (no spec)

- **Cross-process saga.** Wire `Command::PromotePoolWindow` into `agentmux-common::ipc`; revive launcher coordinator; fold host pool-promote into the tear-off saga so the window create/close becomes atomic with srv state. Estimate ~800-1200 LOC, multi-PR.
- **Renderer-registration as saga step.** Change new-window bootstrap so the renderer phones home with `saga_id` after registration completes; saga doesn't `Done` until that fires.
- **Persistent saga state.** Saga journal on disk with replay/compensate on srv restart. Probably waits for Phase G's event-sourced model to subsume it.
- **Phase F host reducer.** Host has FFI handles + Win32 sync constraints that resist the reducer pattern; "host reducer for the easy parts, scaffolding stays for the hard parts" per `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13.

None of these have a written spec yet. When any becomes schedulable, write its spec first; reference §4-§6 of `saga-coordinator-location-analysis-2026-04-30.md` for the gap analysis.

### 7.3 Non-blocking carryovers

- **Codex P2 #608** in `agentmux-launcher/src/ipc/server.rs:438` — `Event::Registered` is appended to the in-memory replay ring before `patch_launcher_identity` runs, so stored events keep reducer sentinels. Documented in `multi-reducer-status-2026-04-29.md` §4. Fold into the next launcher PR that touches `ipc::server` event-emission.

---

## 8. Smoke status — recommended next step

**No smoke testing has been performed on any work shipped this session.** All test coverage is unit-level: 720+ Rust unit tests pass; frontend builds clean. The 6 PRs have not been exercised live in a portable build.

**Deferred twice during the work** (the analysis at the time was "the actual fix isn't shipped yet"). That logic doesn't apply now — the smoke regression IS fixed; this is the highest-value moment to validate.

### What smoke would catch

| Path | Likelihood of issue | What to check |
|---|---|---|
| Tear-off tab → "+" in new window | low — main test of E.5 | the original regression |
| Tear-off block (drag a pane out) → frontend renders | medium — TearOffBlock layout setup is the most stitched-together part | new window shows the block, not blank |
| Drag tab between workspaces | low | dest workspace updates, source updates |
| Drag block between tabs | low | both tabs update, block goes to end |
| Promote block to tab | low | new tab appears as active, block in it |
| Close window | low — straight reducer dispatch | workspace cascade-deleted (current single-window-per-ws behavior) |
| Switch workspace | low | window points at new workspace |
| Tear-off + drop back on origin (RestoreTornOffTab) | medium — saga path, depends on cancel-back hook | tab returns to original index |
| Rename workspace, rename tab, edit object meta | low | persists, reflects in title bar |
| Reorder tabs (drag) | medium — touched by codex P1 #620 carryover | order persists across restart |
| Frontend cold-path orphan recovery (F1.B) | hard to reproduce without failure injection | best-tested via code review |

### Recommendation

**Yes, smoke now.** Build a portable, exercise the table above, fix anything found before piling on more work. Cost ~5-10 min build + ~20 min testing. Higher value than any subsequent work in §7 because everything builds on these PRs.

If smoke is clean, the natural next user-visible work is **E.2c.5b** (renderer dispatcher) which is small and unblocks E.6. Layout migration (E.4) is bigger and has real design questions; defer until smoke confirms the foundation is solid.

---

## 9. Cross-references

- `docs/retro/multi-reducer-status-2026-04-29.md` — sub-phase table (the row-by-row record); use §1-§4 for high-level vision.
- `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` — Path A vs B decision + robustness audit + gap analysis. Read before any saga work.
- `docs/specs/SPEC_PHASE_E_SAGAS_2026-04-30.md` §13 — what E.5 explicitly does NOT close.
- `docs/specs/PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30.md` §9b — F1.A / F1.B follow-ups (now both done).
- `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` — original Phase E spec; §13 sketches Phase F.
- `docs/retro/multi-reducer-proposal-2026-04-28.md` — long-form vision doc.
- `docs/retro/phase-e-status-2026-04-30.md` — superseded by this doc; keep for history.

---

## 10. History of this doc

- **2026-05-01** initial — this doc.
- **2026-05-01** post-smoke addendum — see §11.

---

## 11. Smoke result + drag-UX deferral (2026-05-01)

Built `0.33.533` portable, ran preliminary automated smoke + handed off for human smoke. Findings:

**State-correctness foundation: ✅ confirmed.** Process tree healthy, zero `ERROR`-level log entries, srv-events log clean, persist-subscriber transactions land idempotently, default tab name `tab1` shows (the codex P2 fix from #622 working as intended), 103 MB stable memory.

**Drag UX: ⚠️ pre-existing limitation, NOT a regression from this session.** User test surfaced "tear-off works, but reconnecting a torn-off tab doesn't show a drop zone." Investigation showed:

- Current behaviour: drag a tab, get an HTML5 ghost-tab, release, *then* the torn window appears and follows the cursor via Win32 `SC_MOVE`.
- Original requirement (per `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`): Chrome-faithful flow where the entire window detaches and follows the cursor from the moment of threshold-cross — no ghost tab.
- Spec status: Phase 1 (threshold detection) shipped in PR #559; Phases 2-7 unstarted.
- The current half-HTML5 / half-`SC_MOVE` hybrid is the in-between state the spec replaces wholesale. The cross-window drag mechanism (`start_cross_drag` / `update_cross_drag` / `complete_cross_drag` + `cross-drag-update` / `cross-drag-end` events) is the code path that gets deleted when SC_MOVE drives the drag from threshold-cross.

**Bug observed but not fixed:** `cross-drag-end` fires with empty payload during reconnect attempts, suggesting the source's `handleCrossWindowDragEnd` saw `targetWindow === src` (drop on yourself) and called `cancel_cross_drag` which only emits a minimal payload. Diagnosis is unfinished because the code is marked for deletion. **Do not fix this in the current pipeline** — fixing it would be throwaway work overwritten when the tear-off spec's Phases 2-7 land.

**Smoke deferred for the drag flows** (tear-off, reconnect, cross-window merge, cancel-back). All other smoke matrix items (rename / reorder / promote-block / close-window / switch-workspace) remain valid to run against current code.

What this session DID validate: the reducer/saga foundation is correct under any drag UX. Phase E.5's job was state-correctness; it delivered.

---

## 12. What's next

See `docs/retro/next-steps-2026-05-01.md`.
