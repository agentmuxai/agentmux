# What's next — Architecture-completeness ordering (2026-05-01)

**Date:** 2026-05-01
**Author:** AgentA
**Status:** Forward plan — alternative framing to `next-steps-2026-05-01.md`

---

## Why two next-steps docs

`next-steps-2026-05-01.md` ordered work by **user-visible value first** (tear-off Phase 2 = biggest UX win). This doc orders by **architectural completeness first** — drain the multi-reducer punch list before shipping more features on top.

The decision the user made (2026-05-01): we want the entire reducer architecture in before exotic features and fixes. This doc reflects that choice.

**Companion docs:**
- `reducer-architecture-gaps-2026-05-01.md` — the gap inventory this plan drains.
- `phase-e-status-2026-05-01.md` — current state.
- `next-steps-2026-05-01.md` — alternative framing (UX-first); kept for cross-reference.
- `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — Phase F spec; this plan executes it.

**Header rule:** every step below names *which gap-doc section it closes*. If a step doesn't close a section, it doesn't belong here.

---

## The ordering, at a glance

PR counts below reflect the **tightened plan** (2026-05-01 review): PRs that share infrastructure are merged into single PRs to reduce review surface. The commentary in each step section is the original (more granular) breakdown — keep it for scope-of-work reference; ship per the tightened count.

| # | Step | Closes (gap-doc §) | Estimate | Sub-PRs | Visible to users? |
|---|---|---|---|---|---|
| 1 | F1 — host reducer minimum | §2 (partial), §3 host-pool-promote, §5 (host pipe) | ~850 LOC | **2** (skeleton+pending merged; drag separate) | No (backend-only) |
| 2 | E.6 — renderer multi-source + saga buffering | §1 (E.6), §5 cross-pipe order/version | ~400 LOC | **1** | No (until F1 lights it up) |
| 3 | E.4 — layout reducer migration (Option A) | §1 (E.4), §4 `handle_move_tab` tolerance | ~700 LOC | **2** (Option A; strict-mode flip rides separately) | No |
| 4 | §3 saga durability — durable saga log | §3 saga-state-durability | ~1000 LOC | **1-2** (per spec) | No (unblocks remote-agent sagas) |
| 5 | §4 SQLite-first deletes → sagas | §4 three-deletes-compromise | ~600 LOC | **2** (Block+Tab merged; Workspace separate) | No |
| 6 | F2, F3 — full host reducer per spec | §2 remainder, §3 renderer-registration-as-saga-step | ~1250 LOC | **2** (F.5 + F.6; F.4 folds into tear-off spec) | Yes (drag UX unblocked) |
| 7 | E.7 — integration tests | §1 (E.7) | ~600 LOC | **1** | No |

**Rough total: ~5400 LOC across ~11 PRs.** Multi-week to multi-month depending on cadence.

**Specs to write before code:**
- ✅ `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — Phase F (already written, covers steps 1 + 6).
- 🆕 `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — granularity decision (step 3).
- 🆕 `SPEC_SAGA_DURABILITY_2026-05-01.md` — durable saga log (step 4).
- ❌ Steps 2, 5, 7 — ship from PR-description-grade design notes; no separate spec.

After all 7: Phase G (event-sourced, drop SQLite) remains the long-term ceiling — separate planning effort.

---

## 1. F1 — host reducer minimum

**Closes:** Gap §2 (Phase F implementation — partial), §3 host pool-promote, §5 cross-pipe (host event source ships).

**Spec:** `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §3 (state inventory), §5 (reducer arms), §9 PRs F.1–F.3.

### Scope

The "minimum" host reducer = skeleton + the two cleanest-lifecycle migrations:

1. **F.1 — Host reducer skeleton** (~200 LOC). `host_state::HostState`, `update()` function, dispatch table; all commands return `misrouted_command_error`. Mirrors srv reducer's bootstrap pattern.
2. **F.2 — `pending_window_creations` arm** (~250 LOC). Lowest-risk migration: clean lifecycle (enqueue on create-call, dequeue in `on_after_created`), single producer, single consumer.
3. **F.3 — Drag arms** (`StartHostDrag` / `UpdateHostDrag` / `CompleteHostDrag` / `CancelHostDrag`, ~400 LOC). Replaces today's `commands/drag.rs::start_cross_drag` triad — exactly the API surface the tear-off spec Phase 2 will need to call into.

### Dependencies

- **None** at the API level. Can start tomorrow.
- **Snapshot-and-drop discipline** (Phase F spec §6) must be reaffirmed in code review of every PR — every site that touches the `browsers` lock takes a snapshot, drops the lock, then does Win32 work. CI grep-check / clippy lint should land in F.1 to catch regressions.

### What this leaves un-done

- F.4 (tear-off hook arms) deferred to step 6 — they couple with the tear-off spec's Phase 2 SC_MOVE work and are better folded together.
- F.5 (pool-respawn saga) deferred to step 6 — needs durable saga log (step 4) and full E.6 buffering (step 2) to be visible.
- F.6 (window-cleanup cascade saga) deferred to step 6.

### Exit criteria

- All three PRs landed and on main.
- `--diag host` shows reducer state cleanly (analogous to `--diag srv`).
- Smoke: drag-tab-cross-window + open-new-window flows work identically to before. Pure refactor.
- No new use of `browsers.lock()` outside the snapshot-and-drop pattern.

---

## 2. E.6 — Renderer multi-source + saga buffering

**Closes:** Gap §1 E.6 entry, §5 cross-pipe ordering and per-source version tracking.

**Spec:** `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §6.6 (refines what was scaffolded in E.2c.5b / PR #625).

### Scope

`frontend/util/srv-events.ts` ships scaffolding only — single-source consumption. E.6 layers in:

1. **Per-source version tracking** — `launcher_event_version` and `srv_event_version` (and now `host_event_version` from step 1) tracked separately so dropped events are detectable per pipe.
2. **Saga buffering** — when `SagaStarted { saga_id }` arrives, buffer subsequent saga_id-tagged events in a per-saga queue. Flush atomically on `SagaCompleted`. Renderer atom updates inside a saga are all-or-nothing from the user's perspective.
3. **Resync ordering** — snapshot replay must precede live-event application. Today the renderer accepts whatever order they arrive; E.6 enforces the order.
4. **Force-push protocol spec** — gap-doc §5 calls this out. Decide and document how concurrent live events are handled when the renderer asks for a force-push.

### Dependencies

- **Step 1 (F1) wire format** must be settled — E.6 needs to know host events look like. Can co-develop; doesn't block F.1 from landing first.
- E.2c.5b (PR #625) merged — already done.

### Exit criteria

- Saga: tear-off-tab smoke shows the renderer never displays a half-applied state mid-saga.
- Per-source version tracking visible in `--diag srv` output (or an equivalent).
- Force-push protocol documented in the spec.

---

## 3. E.4 — Layout reducer migration

**Closes:** Gap §1 E.4 entry, §4 `handle_move_tab` migration tolerance (lazy-import + dropped workspace_id check can be tightened once the reducer knows every tab from boot).

**Spec:** sketched in `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md`; the full granularity decision is open.

### Scope — open design question

The minimal slice is straightforward; the full migration depends on a granularity decision:

- **Option A (minimal slice, ~700 LOC):** Migrate `focused`/`magnified` node only. Leave `rootnode` / `leaforder` / `pendingbackendactions` on wcore-direct paths.
- **Option B (full layout, ~1500 LOC):** Migrate everything. Requires a node-path representation so "patch node N's size" is reducer-expressible without dispatching the whole tree per drag-resize tick.

**Recommendation:** ship Option A first as 1-2 PRs, then decide on Option B based on whether the remaining wcore-direct paths cause friction in steps 5-6.

### Dependencies

- None at API level. Can run in parallel with steps 1-2.
- Tightening `handle_move_tab` (the gap-§4 close) waits until the reducer knows every tab — i.e. after Option A *or* B closes.

### Exit criteria

- Option A: focused/magnified mutations route through reducer; everything else unchanged.
- `handle_move_tab` strict-mode flip: drop migration tolerance, reinstate workspace_id check, no longer lazy-import unknown tabs.

---

## 4. Saga durability — durable saga log

**Closes:** Gap §3 "saga state durability" — sagas live in memory, lost on srv crash mid-saga; recovery is best-effort SQLite reconciliation.

**No spec yet.** Write the spec before the first PR.

### Scope

A durable saga log = a SQLite table (or per-saga JSON file) capturing:
- saga_id, saga_kind, started_at
- per-step state: `pending` / `succeeded` / `failed` / `compensating` / `compensated`
- input/output snapshots for each step
- terminal state: `completed` / `failed` / `compensated`

On srv start: scan log for `pending` sagas; resume them via the saga coordinator.

### Why this matters

Today's saga set (tear-off, restore, promote) completes in milliseconds — durability is a nice-to-have. The killer use case is **long-running sagas** (gap-§3): "spawn a remote agent, wait for it to register, attach it to a tab" can take seconds-to-minutes. Today a srv crash drops that on the floor; durability turns it into a recoverable workflow.

### Dependencies

- **Saga coordinator** (already shipped in Phase E).
- **Persist subscriber** (already shipped in F1.A).
- No dependency on F1 or E.6 — can run earlier if a remote-agent feature shows up.

### Exit criteria

- Spec written.
- Saga log table + write path land in PR 1 (~500 LOC).
- Resume-on-restart logic + recovery tests in PR 2 (~500 LOC).
- E.7 will add proptests for crash-mid-saga recovery.

### Risk

This is the biggest *design* unknown in the punch list. The other steps are largely "do the obvious thing." Durability has real choices: synchronous-write-per-step vs WAL, JSON vs structured columns, retention policy. Plan for spec churn.

---

## 5. SQLite-first deletes → sagas

**Closes:** Gap §4 "three SQLite-first deletes" — DeleteBlock, DeleteWorkspace, DeleteTab apply to SQLite first, then emit reducer commands. They short-circuit the reducer pattern.

### Scope

Convert each delete into a saga that:
1. Validates the delete is allowed (against reducer state).
2. Computes the cascade (which children get cleaned up, in what order).
3. Applies the cascade through the reducer.
4. Persist subscriber writes to SQLite (existing pattern).

Three sagas, one PR each:
- **DeleteBlockSaga** (~200 LOC).
- **DeleteWorkspaceSaga** (~250 LOC) — largest cascade.
- **DeleteTabSaga** (~150 LOC).

### Dependencies

- **Saga durability (step 4)** is **not** a hard prerequisite — these sagas complete in milliseconds. But landing 4 first means deletes are crash-recoverable from day one.
- **F1 (step 1)** strongly recommended first — DeleteWorkspace cascades may want to coordinate with host-side cleanup (close associated CEF windows), which requires the host pipe.

### Exit criteria

- All three SQLite-first paths removed from `service.rs`.
- Cascade ordering verified by proptest (gap-doc §1 E.7).
- Smoke: deleting a workspace cleanly tears down all blocks and tabs.

---

## 6. F2, F3 — Full host reducer (tear-off + sagas)

**Closes:** Gap §2 (Phase F implementation — remainder), §3 "renderer registration as saga step."

**Spec:** `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §9 PRs F.4–F.6.

### Scope

- **F.4 — Tear-off hook arms** (~300 LOC). Couples with tear-off spec Phase 2 (SC_MOVE handshake). **Decide during scoping**: fold F.4 into the tear-off spec's Phase 2 PRs, or land F.4 standalone first.
- **F.5 — Pool-respawn-on-promote saga** (~250 LOC). First real cross-process saga (launcher↔host). Revives launcher-side coordinator from E.1a (currently framework-stub).
- **F.6 — Window-cleanup cascade saga** (~300 LOC). Subsumes today's implicit cascade in `wcore::close_window` + multiple host close paths.
- **Renderer registration as a saga step** (gap-doc §3) — fold into F.4 or F.5 as appropriate.

### Dependencies

- **Step 1 (F1)** mandatory.
- **Step 2 (E.6)** mandatory — saga buffering is what makes cross-process sagas visible end-to-end.
- **Step 4 (saga durability)** strongly recommended — F.5 / F.6 are the first "real" saga consumers in production; durability prevents losing them on srv crash.
- **Tear-off spec Phase 2** decision (fold-in vs separate) — open.

### Exit criteria

- Drag UX unblocked — tear-off + restore + promote all flow through host reducer + saga coordinator.
- Pool-respawn p99 visible in telemetry (gap-doc §3 first appearance of cross-process saga timing risk).
- No more module-level Mutexes in `agentmux-cef/src/commands/drag.rs` or `commands/tear_off_hook.rs`.

---

## 7. E.7 — Integration tests (Phase E exit)

**Closes:** Gap §1 E.7 entry — proptests landed in PR #627; integration tests are what's left.

### Scope

- **End-to-end saga tests** — drive a saga through srv + host stubs, assert final state across both reducers.
- **Cross-pipe ordering tests** — verify renderer's per-source version tracking handles interleaved + delayed events correctly.
- **Recovery-from-crash tests** — saga partway through, kill srv, restart, assert resume-or-rollback (depends on step 4).
- **Optional: load test** — N concurrent sagas, assert no state divergence, no leaked sagas.

### Dependencies

- **Step 1 (F1)** — host reducer must exist for end-to-end tests to be meaningful.
- **Step 2 (E.6)** — cross-pipe ordering tests need the renderer's multi-source dispatcher.
- **Step 4 (saga durability)** — recovery-from-crash tests depend on it.

### Exit criteria

- Phase E formally closed.
- Test suite runs in CI.
- `--diag srv`, `--diag host`, `--diag sagas` (in-flight registry) all available and tested.

---

## What sits outside this plan

- **Phase G (event-sourced, drop SQLite)** — the architectural ceiling. Sketched in older docs; needs its own spec; estimated months of effort. Don't start until 1-7 land and the event log is exercising real load.
- **Tear-off spec Phases 2-7** (Chrome-faithful drag UX) — `next-steps-2026-05-01.md` has this as #1. In *this* plan it slots into step 6 (tear-off hook migration overlaps Phase 2 SC_MOVE work). Phases 3-7 of the tear-off spec remain feature work, not architecture work, and run after this plan completes.
- **Platform parity (`--diag` Windows-only, Wayland deferred)** — gap §6. Not architecture-shaped; address opportunistically when a non-Windows user asks.
- **`merge_meta_patch` validation** — gap §4. Small. Can fold into any step that touches the meta-update path.

---

## How to execute this plan

**Cadence:** one step at a time, fully closed before starting the next. The dependency graph allows parallel work in some places (3 with 1, 4 with 1) but sequencing them serially keeps review load bounded and prevents half-finished migrations from accumulating.

**Per-step ritual:**
1. Open a tracking issue or follow-up doc for the step.
2. Land sub-PRs in the order this doc suggests.
3. After all sub-PRs land, write a one-page status note (analogous to `phase-e-status-2026-05-01.md`) confirming the gap section is closed.
4. Update `reducer-architecture-gaps-2026-05-01.md` to strike the closed gaps.
5. Move to the next step.

**Stop conditions** — if any of these surface mid-plan, pause and re-plan:
- A user-facing regression that this plan won't catch for ≥2 steps.
- A discovered design assumption in the gaps doc that turns out to be wrong.
- A spec gap large enough that a future step's LOC estimate doubles.

**Don't-do list:**
- Don't add new sagas or new reducer arms outside the plan unless they close a gap-doc section.
- Don't ship F.4 without first deciding fold-in vs standalone with the tear-off spec.
- Don't reinstate `handle_move_tab` strict mode until step 3's Option A or B closes — it'll break bootstrap.

---

## Cross-references

- `reducer-architecture-gaps-2026-05-01.md` — gap inventory (what this plan drains).
- `next-steps-2026-05-01.md` — alternative framing (UX-first).
- `phase-e-status-2026-05-01.md` — current state.
- `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — the F-spec this plan executes.
- `saga-coordinator-location-analysis-2026-04-30.md` — saga design decisions.
- `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` — original Phase E spec; still load-bearing for §6.6 (E.6 details).
