# Saga coordinator — location decision and robustness audit

**Date:** 2026-04-30
**Status:** Decision input for PR 3 (E.5.5+6 tear-off sagas).
**Audience:** anyone implementing or reviewing PR 3 / PR 4, or scoping Phase F.

---

## 0. TL;DR

1. **The original spec put the saga coordinator in the launcher.** That call was correct at the time — the original tear-off saga genuinely fanned out across host pool + launcher windows + srv state.
2. **The actual implementation took a different cross-process orchestration path.** `Command::PromotePoolWindow` was never wired into `agentmux-common::ipc`; instead the frontend was wired to call CEF host's pool-promote IPC handler directly. The pool itself is alive — only the launcher↔host orchestration wire was never built.
3. **Combined with PR #619's srv-level Window concept**, every saga in the E.5 plan (7 of them) now mutates only srv state. Cross-process work is frontend-orchestrated.
4. **Recommended: build the saga coordinator in srv (Path A).** In-process dispatch via oneshot channel; the new saga spec already assumes this. The launcher coordinator stays as a labeled stub for hypothetical future cross-process sagas.
5. **E.5 closes the smoke regression but is a partial robustness improvement.** Four robustness gaps remain: per-step SQLite transactions (no plan), host pool-promote outside saga (Phase F stub), renderer registration outside saga (no plan), saga persistence (Phase F stub). Two of them (#1 and frontend orphan-cleanup on host failure) are cheap follow-up PRs that should be filed.

---

## 1. Why the launcher was the original answer

The original Phase E spec (`SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §7.1) was unambiguous:

> "Phase E adds a **centralized saga coordinator in the launcher** that owns saga lifecycle."

The reasoning shows up in the worked example (§7.2 — TearOffBlock):

| Step | Target pipe | Operation |
|---|---|---|
| 1 | **Host** | `Command::PromotePoolWindow{}` — promote a pool CEF window |
| 2 | **Srv**  | `Command::CreateWorkspace{name}` |
| 3 | **Srv**  | `Command::CreateTab{workspace_id, ...}` |
| 4 | **Srv**  | `Command::MoveBlockToTab{...}` |
| 5 | **Launcher** | `Command::AssignWorkspaceToWindow{...}` |

A saga that genuinely fans out across three reducers in three processes can really only live in the central pipe-routing process — the launcher. The `PipeTarget` enum in `agentmux-launcher::saga` (`LauncherSelf | Host | Srv`) baked that in.

The original answer wasn't arbitrary. It tracked the actual data flow.

---

## 2. What changed

### 2.1 `Command::PromotePoolWindow` was never wired through the launcher

For step 1 to flow through a saga, the wire protocol between launcher and host had to carry `PromotePoolWindow` as an IPC command. **That command was never added to `agentmux-common::ipc`** — a grep for `PromotePoolWindow` in `agentmux-common/src/` and `agentmux-launcher/src/` returns nothing.

What was built instead: the frontend calls the CEF host's IPC handler directly. Per `agentmux-cef/src/commands/drag.rs:394`:

```
requestTearOff (tabbar.tsx):
  1. WorkspaceService.TearOffTab        ← srv RPC (frontend → srv pipe)
  2. tear_off_pool_promote               ← host IPC (frontend → CEF host
                                           Tauri-style handler, NOT via
                                           the launcher pipe)
  3. tear_off_sc_move_handshake          ← host IPC (Win32 SC_MOVE)
```

The pool itself is still in use (`agentmux-cef/src/commands/window_pool.rs` is alive — pre-warmed CEF windows promote in ~0 ms vs ~150-300 ms cold). What's gone is the **orchestration shape** that would have made it a saga step. The frontend is the cross-process orchestrator today, holding both `srv-rpc` and `host-rpc` clients in the same call site.

### 2.2 PR #619 added a srv-level `Window` concept

E.5.1+2 introduced `WindowRecord` + `state.windows: HashMap<window_id, WindowRecord>` in the srv reducer, plus `Command::CreateWindow{window_id, workspace_id}` and `Event::SrvWindowOpened{...}`. These track the AgentMux-level "which workspace is this window viewing" — distinct from the launcher's notion of "which Win32 HWND has this label."

That collapsed step 5 of the original saga. Workspace-to-window assignment is now a srv reducer mutation, not a launcher one.

### 2.3 Result

Every saga in the E.5 plan now mutates only srv state:

| Saga | Steps | Pipes touched |
|---|---|---|
| TearOffTab | CreateWorkspace → MoveTab → CreateWindow | srv only |
| TearOffBlock | CreateWorkspace → CreateTab → MoveBlock → CreateWindow | srv only |
| RestoreTornOffTab | MoveTab → DeleteWorkspaceCascade | srv only |
| MoveTabSaga (PR 4) | MoveTab | srv only |
| MoveBlockSaga (PR 4) | MoveBlock | srv only |
| CreateWindowSaga (PR 4) | CreateWorkspace → CreateWindow | srv only |
| CloseWindowSaga (PR 4) | CloseWindowInternal → cond. DeleteWorkspaceCascade | srv only |

Not because cross-process work has gone away — it's still there, as the frontend's two-step `srv-rpc + host-rpc`. But the **piece** that needs *coordinated state-machine retries with compensation* (the saga's value proposition) is contained entirely within srv state. Pool-promote isn't transactional with anything; the frontend can retry it on its own without saga ceremony. The srv-side multi-step is what needs atomicity.

---

## 3. Path comparison

### Path A — coordinator in srv

Build a saga coordinator inside `agentmux-srv` (`agentmux-srv/src/sagas/`). It owns:
- `next_saga_id: AtomicU64`
- `in_flight: HashMap<u64, (Box<dyn Saga>, oneshot::Sender<SagaOutcome>)>`
- A handle to the srv reducer for in-process dispatch
- A subscription to the srv broadcast bus

RPC handlers register sagas via `dispatch_saga(state, saga).await`. The launcher coordinator becomes a labeled stub.

This is what `SPEC_PHASE_E_SAGAS_2026-04-30.md` §4.5 already assumes (the spec literally writes `state.saga_coordinator.register(...)` — `state` here is srv state).

### Path B — coordinator in launcher (per original spec)

Keep the coordinator in `agentmux-launcher::saga`. Srv RPC handlers reach it via cross-process IPC: srv sends a new `Command::StartSaga{kind, args}` up the launcher pipe; launcher routes events back over the broadcast bus; srv RPC handler awaits a saga-correlated event over the bus.

Requires:
- A new `Command::StartSaga` over IPC.
- Either launcher knowing about every saga shape (kind/args) — coupling launcher to srv-specific commands — or a generic "execute this serialized saga state" mechanism (more abstract, more code).
- Saga-driven commands flowing srv → launcher → srv (round-trip latency on every step).

### Path C — both coordinators

Srv coordinator handles srv-only sagas. Launcher coordinator stays available for genuine cross-reducer sagas. Adds duplicated infrastructure for unclear future benefit.

### Trade-off table

| Concern | Path A (srv) | Path B (launcher) | Path C (both) |
|---|---|---|---|
| Lines of new code (PR 3) | ~600 (coordinator + 3 sagas) | ~900 (coord + sagas + IPC + wire types) | ~700 |
| Existing E.1a code | becomes labeled stub | becomes the real implementation | both used |
| Latency per saga step | in-process oneshot | IPC round-trip | depends |
| Failure modes | reducer mutex contention | IPC pipe failures × 2 | both |
| Future cross-reducer saga | needs coord wiring (build later) | already supported | already supported |
| Faithfulness to original spec | low | high | high |
| Faithfulness to current saga spec (4-30) | high | low | mixed |

**Recommendation: Path A.**

Rationale:
1. **Every saga in the E.5 plan is srv-only.** Building cross-process coordination machinery for zero cross-process consumers is YAGNI.
2. **The architectural justification for the original launcher placement no longer holds.** `Command::PromotePoolWindow` was never wired; PR #619 collapsed step 5 into srv. What remains is purely srv-side multi-step state.
3. **In-process dispatch is materially simpler.** A oneshot channel beats an IPC round-trip per saga step.
4. **The new spec already says srv.** Building where the spec lives is the faithful execution.
5. **The launcher coordinator is reversible.** If Phase F surfaces a saga that genuinely spans processes, we revive `agentmux-launcher::saga` then. Today it's framework-only — leaving it as a stub costs nothing.

---

## 4. Robustness audit

### 4.1 Failure modes during a tear-off

| # | Step | What can fail | Today (wcore-direct) | Path A (srv coordinator) |
|---|---|---|---|---|
| 1 | Source workspace lookup | tab/workspace not found | Errors before mutating; safe | Reducer rejects with `Error` event; safe |
| 2 | Update source workspace (remove tab, reassign active) | SQLite write error | **Bug today**: wcore has no transaction wrapping; partial state can survive | Reducer in-memory mutation is atomic; subscriber's SQLite write is a single `update`. If it fails, saga compensates by re-adding tab to source |
| 3 | Create new workspace | UUID collision (impossible), SQLite insert error | If this fails after step 2, the tab is gone from source — **state is corrupted: tab orphaned**. wcore returns error but partial state survives | Saga sees `Error` event from reducer/subscriber; runs compensate → MoveTab back to source. Atomic from caller's perspective |
| 4 | Frontend opens CEF window | pool exhausted (handled via cold path), cold-path window-create fails | Srv state already mutated; host failure leaves orphan workspace nobody can see | **Same as today.** Saga doesn't include host. Frontend handles fallback or surfaces error; orphan workspace persists |
| 5 | Renderer registers `(window_id, workspace_id)` mapping | network/IPC error after CEF window opened | Srv has workspace + tab; host has CEF window; mapping missing → orphan window pointing nowhere | Mapping registration is its own RPC; saga doesn't span it. Same outcome as today |
| 6 | Mid-saga srv crash | SIGSEGV, panic | Partial state survives in SQLite; reducer rebuilds from bootstrap | In-flight saga is lost; reducer rebuilds from event log + SQLite. **Compensation not run**: partial state survives. Phase F adds saga persistence |

### 4.2 What Path A guarantees vs not

**Guarantees:**
- ✅ Srv reducer state and SQLite stay consistent at the boundary of each saga step (subscriber's idempotent apply).
- ✅ Saga compensation runs on srv-side step failure (rows 1-3 above).
- ✅ The reducer/wcore-divergence smoke regression is fixed (the actual user-visible bug).
- ✅ saga_id correlation lets the renderer (E.6) buffer events atomically.

**Does NOT guarantee:**
- ❌ Each saga step's SQLite writes are in a single transaction. Today's wcore code isn't either (e.g., `tear_off_tab` does two `store.update`/`store.insert` calls non-transactionally). Mitigation: subscriber arms are idempotent and bootstrap tolerates partial state. Strict atomicity per step needs explicit `wstore` transactions — orthogonal to coordinator placement.
- ❌ Host pool-promote is included in the saga. If the CEF window fails to open, srv-side state was already mutated and isn't rolled back. Frontend must compensate (call `DeleteWorkspaceCascade`) or accept the orphan.
- ❌ Renderer-side `(window_id, workspace_id)` registration is included. If the new window's renderer dies before calling that RPC, srv has no mapping for the window.
- ❌ Saga state is durable. Mid-saga srv restart abandons in-flight work and runs no compensation.

### 4.3 Tab vs pane (block) tear-off

Identical failure structure, only the step count differs:

| Saga | Steps | Compensation chain (max length) |
|---|---|---|
| TearOffTab | CreateWorkspace → MoveTab → CreateWindow | 3 reverse commands |
| TearOffBlock | CreateWorkspace → CreateTab → MoveBlock → CreateWindow | 4 reverse commands |

Both fully covered by Path A's srv-only compensation. Both have the same gap at steps 4-5 (host pool-promote + renderer registration outside the saga).

### 4.4 Robustness gradient

```
today (wcore-direct, non-transactional, no compensation)
  → Path A (or Path B) — fixes the smoke regression; same robustness ceiling
  → above + per-step SQLite txns in subscriber  ← cheap follow-up
  → above + frontend orphan-cleanup on host failure  ← cheap follow-up
  → above + Command::PromotePoolWindow wired into saga  ← cross-process saga
  → above + persisted saga state  ← Phase F+ target
```

Choosing between A and B for E.5 is a question about **dispatch mechanics, code locality, and future extensibility**, not about state preservation. Path A and Path B have the same end-to-end robustness ceiling.

---

## 5. Are the gaps planned for?

Honest audit. §4.4 names six robustness levels; here's whether each beyond Path A is actually planned:

| Gap | Plan? | Where it's discussed | Concrete spec? |
|---|---|---|---|
| Per-step SQLite transactions in subscriber | ❌ **No plan** | Not mentioned in `SPEC_PHASE_E_SAGAS_2026-04-30.md`, `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md`, or the execution plan. Grep for `transaction\|BEGIN\|COMMIT` in saga/reducer specs returns zero hits. | None |
| Frontend orphan-cleanup on host failure | ❌ **No plan** | Not mentioned. `drag.rs:394` comment treats the two RPCs as independent; there's no error-recovery design. | None |
| Host pool-promote as saga step | ⚠️ **Stub deferral** | `PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30.md` §9: "Cross-process saga correlation — Phase F or beyond." | **No Phase F spec exists.** |
| Renderer registration as saga step | ❌ **No plan** | Not mentioned anywhere. | None |
| Persistent saga state across restart | ⚠️ **Stub deferral** | `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §15 mentions it once: "revisit if Phase F's pool-respawn sagas need it." | None |
| Saga timeout / retry | ✅ Planned | `SPEC_PHASE_E_SAGAS_2026-04-30.md` §4.5: 5s default, tunable. | Yes — implemented in PR 3. |

### 5.1 What "Phase F or beyond" actually means

Phase F has **no written spec**. It's sketched in three places:

1. `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13 — "Phase F preview" — frames Phase F's job as the **host reducer**, with explicit caveats that `browsers` (CEF FFI) and pool maps "resist the reducer pattern":
   > Phase F's outcome is likely "host reducer for the easy parts, scaffolding stays for the hard parts."

2. `multi-reducer-proposal-2026-04-28.md` — high-level "third reducer, retire scaffolding model" intent only.

3. The phrase "Phase F or beyond" appears 5+ times in the saga spec as a deferral cookie.

**No spec schedules or scopes the cross-process saga work, the per-step transaction work, the renderer-registration work, or saga persistence.**

### 5.2 Honest characterization of the current arc

```
PR 1+2 (#619, #620) — done
PR 3+4 (E.5)        — fixes the smoke regression by routing srv-state
                      through reducer + sagas
                      ↓
                      leaves four gaps documented in §4.2
                      ↓
[no concrete plan for items 1-4 below]
                      ↓
some future phase   — closes them
```

E.5 is a **partial** robustness improvement, not a complete one. Anyone who reads the plan and infers "tear-off becomes transactional after E.5" is mistaken: tear-off becomes srv-side-atomic. Cross-process orphans (CEF window opens but srv state is broken; or srv state succeeds but renderer fails to register) remain possible.

The actual user-visible value of E.5 is:
1. Fixing the smoke regression (reducer/wcore divergence).
2. The saga_id correlation infrastructure that unblocks E.6 (renderer multi-source buffering).

---

## 6. Plan forward

### 6.1 PR 3 (E.5.5+6) — proceed with Path A

Build the saga coordinator in `agentmux-srv/src/sagas/`. Implementation sketch:

**Module layout:**
```
agentmux-srv/src/sagas/
  mod.rs                     // Saga trait + SagaStep + dispatch_saga()
  coordinator.rs             // SagaCoordinator (registry + bus subscription)
  tear_off_tab.rs            // TearOffTabSaga state machine + tests
  tear_off_block.rs          // TearOffBlockSaga state machine + tests
  restore_torn_off_tab.rs    // RestoreTornOffTabSaga state machine + tests
```

**Trait shape** (per `SPEC_PHASE_E_SAGAS_2026-04-30.md` §4.3):
```rust
pub trait Saga: Send + 'static {
    fn saga_id(&self) -> u64;
    fn name(&self) -> &'static str;
    fn start(&mut self) -> Vec<Command>;
    fn on_event(&mut self, event: &Event) -> SagaStep;
    fn compensate(&mut self) -> Vec<Command> { Vec::new() }
}
```

**Coordinator** subscribes to srv broadcast bus, maintains in-flight registry, dispatches commands via reducer in-process, emits `SagaStarted/Completed/Failed` lifecycle events.

**Dispatch from RPC handler** uses oneshot + 5s `tokio::time::timeout`.

**Wire-up:** `AppState` gains `saga_coordinator: Arc<SagaCoordinator>`. `main.rs` constructs after reducer+bus exist; spawns run-loop. RPC handlers in `service.rs` (TearOffTab, TearOffBlock, RestoreTornOffTab) replace `wcore::tear_off_*` calls with `dispatch_saga(...)`.

**`saga_id` threading:** add `saga_id: Option<u64>` to every Command and Event. Reducer copies through. Coordinator sets on outgoing commands.

**Existing launcher coordinator:** add a comment pointing to this doc, leave the code in place. PR 4 cleanup may delete it; safer to keep through E.5.

### 6.2 PR 4 (E.5.7+8+9) — finish saga RPC migration + cleanup

Per existing execution plan §5. No design changes needed.

### 6.3 Follow-up PR — subscriber SQLite transactions

**Scope:** wrap each `apply_*_event` arm's writes in a `wstore` transaction. Subscriber currently uses sequential `store.update`/`store.insert` calls; if the second of two writes fails, partial state survives.

**Estimate:** ~200 LOC, 1 PR, no architectural risk.

**When:** opportunistic — could ship inside PR 4 (low extra LOC) or as a focused follow-up. Not blocking E.5.

**Why now:** this is a real correctness gap that already exists (predates E.5). Cheapest robustness win on the table.

### 6.4 Follow-up PR — frontend orphan-cleanup on host failure

**Scope:** in `tabbar.tsx::requestTearOff` (and equivalents for block tear-off), if `tear_off_pool_promote` returns `pool_exhausted` AND `open_window_at_position` then fails, dispatch `WorkspaceService.DeleteWorkspaceCascade` to clean up the orphan workspace just created via `TearOffTab`. Show a user-facing error toast.

**Estimate:** ~50 LOC TS, 1 PR.

**When:** post-E.5. Doesn't block anything.

**Why now:** trivial fix for a real failure mode that produces visible junk in the workspace list.

### 6.5 Phase F-sized — defer until Phase F has a spec

Three items need spec work before they can be scheduled:

1. **Cross-process saga (`Command::PromotePoolWindow` wired)** — wires host pool-promote into the saga. Closes most of gap 4.4-#4.
2. **Renderer-registration as saga step** — closes gap 4.4-#5.
3. **Persistent saga state** — closes gap 4.4-#6 (mid-saga crash). Probably the right answer is to wait until Phase G's event-sourced model lands and let the journal subsume saga persistence.

Action: when Phase F is scheduled, write its spec first; reference §4 of this doc for the gap analysis and proposed shape.

---

## 7. Open questions (carried from earlier draft)

1. **Saga timeout.** Spec says 5s default. Reasonable for local in-process flows; a tear-off should complete in tens of milliseconds. Keep 5s.
2. **Saga restart on srv crash.** Spec defers persistence to Phase F. In-flight sagas die on srv restart; renderer-side timeout handles it. PR 3 doesn't need to solve this.
3. **Multiple in-flight sagas same saga_id.** Coordinator allocates monotonically; collisions impossible within one srv run.
4. **Saga events arriving before SagaStarted is published.** The coordinator emits SagaStarted FIRST (synchronously, holding the registry lock), THEN dispatches start()'s commands. So step-1 events always have a corresponding SagaStarted ahead of them on the bus.

---

## 8. Decision

**Pending user confirmation. Default proceed: Path A + file follow-ups §6.3 and §6.4 as separate PRs after E.5 closes.**

History of this doc:
- Initial draft claimed "the pool was removed" — corrected; the pool is alive, only the launcher↔host orchestration wire was never built.
- Initial draft claimed Path A vs B was a robustness question — corrected; both have the same end-to-end robustness ceiling.
- Initial draft claimed the gaps were "Phase F territory" — corrected; that's a deferral cookie, not a plan. Phase F has no spec.
