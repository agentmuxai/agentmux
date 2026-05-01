# Saga Architecture — Execution Plan

**Date:** 2026-05-01
**Specs:**
- [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) (target)
- [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md) (CPD)
- [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md) (LSD)

This plan converts the two implementation specs into a sequenced + parallelizable PR list. Total effort: ~1500 LOC across 9 PRs, plus ~200 LOC of integration tests + diagnostic CLI updates.

The plan is structured so the autonomous PR sequencer (the same one that drove batch 1+2 of Phase F+G) can execute it: each PR has a self-contained agent brief, explicit dependencies, and a green-light condition.

---

## 1. Summary table

| # | PR | Spec | LOC | Depends on | Parallel-safe with |
|---|----|------|-----|------------|---------------------|
| 1 | CPD-1 schema additions | CPD §4 PR1 | 150 | — | LSD-1, CPD-2 |
| 2 | CPD-2 host pipe wrapper | CPD §4 PR2 | 300 | — | LSD-1, CPD-1 |
| 3 | LSD-1 saga log + API | LSD §4 PR1 | 250 | — | CPD-1, CPD-2 |
| 4 | CPD-3 wire saga dispatch | CPD §4 PR3 | 200 | CPD-1, CPD-2 | LSD-2 |
| 5 | LSD-2 coordinator integration | LSD §4 PR2 | 200 | LSD-1 | CPD-3 |
| 6 | CPD-4 per-saga correlation | CPD §4 PR4 | 150 | CPD-3 | LSD-3, CPD-5 |
| 7 | CPD-5 host-side saga_id LRU | CPD §4 PR5 | 100 | CPD-3 | LSD-3, CPD-4 |
| 8 | LSD-3 recovery walker + --diag | LSD §4 PR3 | 150 | LSD-2 | CPD-4, CPD-5 |
| 9 | LSD-4 retention vacuum | LSD §4 PR4 | 50 | LSD-1 | anything |

Total: 9 PRs, ~1550 LOC.

---

## 2. Batch structure

Three parallel batches. Each batch fans out to multiple background agents working in worktrees, then converges before the next batch starts (mirroring the Phase F+G batch 2 pattern that landed cleanly).

### Batch 1 — Foundations (3 parallel agents, ~700 LOC)

**Goal:** lay schema + infra that everything else depends on. No behavior change yet.

- **Agent 1: CPD-1 schema additions** — add `saga_id` to host-bound Commands (`SpawnPoolWindow`, `ReapPanes`, `DrainPoolIfLast`), add `saga_id: Option<u64>` to corresponding `Report*` Commands and Events, add `HostFrame` envelope, add `Command::ReportSagaActionFailed` + `Event::SagaActionFailed`. `#[serde(default)]` on new fields for one release of forward-compat. Update reducer arms to plumb saga_id. Tests for serialization round-trips.
- **Agent 2: CPD-2 host pipe wrapper** — new module `agentmux-launcher/src/host_pipe/` with `HostPipe { send_command, send_event, run_connection_loop }`. Bounded `pending_buffer: VecDeque<HostFrame>` (cap 64). Reconnect loop; on host disconnect >30s drop pending buffer + emit `SagaActionFailed` for affected sagas. Refactor existing event fanout through `HostPipe::send_event`. Unit tests with mock pipe (drop/hold/reorder). NO saga-side wiring yet — `apply_action` for `Host` still log-only.
- **Agent 3: LSD-1 saga log + API** — new module `agentmux-launcher/src/saga/log/` with `LauncherSagaLog` struct, schema migrations, all CRUD methods (`start_saga`, `terminate_saga`, `start_step`, `finish_step`, `fail_step`, `unresolved_sagas`, `mark_failed_compensation`, `max_saga_id`, `snapshot_recent`, `vacuum_older_than`). SQLite at `~/.agentmux/launcher-sagas.db`, WAL mode, 5s busy timeout. Mirror srv `SagaLog` API where possible. NO coordinator integration yet.

**Convergence:** all 3 PRs merge. Then batch 2 starts.

**Why parallel-safe:** zero shared file edits. CPD-1 touches Command/Event schema + reducer arms; CPD-2 touches saga_coordinator.rs + new module; LSD-1 is entirely new files.

### Batch 2 — Integration (2 parallel agents, ~400 LOC)

**Goal:** wire the foundations together. After this, sagas drive real host-side work and coordinator writes to durable log.

- **Agent 4: CPD-3 wire saga dispatch** — `apply_action` for `IssueCmd::Host` now calls `host_pipe.send_command()` instead of logging. Add `inject_saga_id()` exhaustive helper. Add `timeout()` method to `Saga` trait (default 5s, F.6 overrides 30s). Coordinator listens for `Event::SagaActionFailed` and terminates matching saga. F.5 + F.6 stop being narrators. Integration test: kill -9 host mid-saga → saga terminates `Failed { reason: "host pipe send failed" }`.
- **Agent 5: LSD-2 coordinator integration** — wrap in_flight sagas in `InFlightSaga { saga, awaiting_step }`. Coordinator calls `start_saga` / `terminate_saga` / `start_step` / `finish_step` / `fail_step` at the right lifecycle points. New `Saga::input_snapshot()` method (default returns `null`). Tests verify a saga's full lifecycle leaves expected log state.

**Convergence:** both PRs merge. Soak smoke-test (open + close 5 windows, watch logs) before batch 3.

**Why parallel-safe:** CPD-3 edits saga/mod.rs around the dispatch path. LSD-2 also edits saga/mod.rs but around the coordinator's lifecycle hooks. Risk of merge conflict is real — pick one to land first (recommend CPD-3 because it's the bigger change), rebase the other.

**If conflict:** the second agent's PR will fail CI on rebase; reagent will flag it; agent rebases manually + re-pushes (same pattern as #639 round 2 in batch 2).

### Batch 3 — Refinements (3 parallel agents, ~350 LOC + small)

**Goal:** retire the F.5/F.6 evict-and-replace workaround, add operator visibility, add retention.

- **Agent 6: CPD-4 per-saga correlation** — saga `on_event()` filters by `event.saga_id == self.expected_saga_id`. Remove evict-and-replace policy from `match_trigger()`. Concurrent-promote and concurrent-window-close test cases now pass without false-positive saga failures. Remove the `pool_respawn::promoted_label` `#[allow(dead_code)]` (it's now used).
- **Agent 7: CPD-5 host-side saga_id LRU** — host code (`agentmux-cef/src/...`) gains an idempotency LRU keyed by `(saga_id, command_kind)`, bound 256. Test: send same command twice → second send re-emits same Report.
- **Agent 8: LSD-3 recovery walker + --diag sagas** — in `main.rs` before `run_coordinator`, call `compensate_unresolved()` analog. Marks unresolved sagas `failed_compensation { reason: "launcher restart" }`. Extend `--diag sagas` to query both srv and launcher saga logs, formatted printer for step rows. Integration test: simulate crash by abruptly dropping coordinator mid-saga → restart → verify saga marked failed_compensation.

**Convergence:** all 3 merge. Final smoke test with --diag verifies operator visibility.

**Why parallel-safe:** CPD-4 touches saga code, CPD-5 touches host code (different crate), LSD-3 touches main.rs + diag code (different file).

### Batch 4 (or 3.5) — Trivial (1 agent, ~50 LOC)

- **Agent 9: LSD-4 retention vacuum** — startup call to `vacuum_older_than(7 days)` against launcher saga log. Configurable retention via `~/.agentmux/config.toml`. Trivial PR; can land any time after LSD-1.

Could ride along in batch 3 if there's bandwidth, but it's also fine as a solo follow-up.

---

## 3. Critical path

```
LSD-1 ──▶ LSD-2 ──▶ LSD-3
                       │
                       ▼
                 (all merged)

CPD-1 ──┐
         ├──▶ CPD-3 ──▶ CPD-4
CPD-2 ──┘     │
              └──▶ CPD-5
```

Critical path length: 3 sequential PRs (CPD-1 → CPD-3 → CPD-4). At ~1 hour per PR cycle (open + 1-2 review rounds + merge) we're looking at ~3 hours minimum wall-clock for the bottleneck path, with batches 1+3 parallelizing the rest.

---

## 4. Sequencing rationale

**Why CPD-1 (schema) before CPD-2 (host pipe wrapper)?** Actually they're parallel. CPD-2 doesn't strictly need the new schema fields — the wrapper is generic over `HostFrame`. But CPD-3 needs both. So we batch them.

**Why LSD before CPD-3?** LSD-1 must land before CPD-3 wires saga dispatch, because once dispatch is real, crashes mid-saga become observable, and we want durable evidence of those crashes. Without LSD, we'd ship a window where dispatch works but recovery doesn't. That window is short (until LSD-2 lands) but it'd be an awkward soak.

**Why LSD-2 in parallel with CPD-3 (batch 2)?** LSD-2 has no runtime effect until LSD-3 lands and reads the log on startup. So shipping LSD-2 ahead of LSD-3 just populates a log nobody reads. That's safe — strictly additive. Pairing LSD-2 with CPD-3 means by end-of-batch-2 we have wire + log; batch 3 connects them.

**Why retire evict-and-replace (CPD-4) only after CPD-3 lands?** Evict-and-replace is the *workaround* for missing per-saga correlation. The proper fix needs `saga_id` carried end-to-end, which lands in CPD-1 + propagates through CPD-3. Removing the workaround before the proper fix is in place would re-expose the concurrent-saga bug.

---

## 5. Risk register

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| R1 | CPD-1 schema breaks running host instances | Low | Med | `#[serde(default)]` on new fields; soak through one release before requiring them |
| R2 | CPD-2 + LSD-2 merge-conflict on saga/mod.rs | Med | Low | Land batch 2 PRs serially if conflict surfaces; reagent will catch on rebase |
| R3 | CPD-3 reveals host bugs that didn't matter when saga was a narrator | Med | Med | Integration test before merge; staged rollout (smoke test on dev box for 24h) |
| R4 | LSD-2 coordinator hold-mutex too long during start_step writes | Low | Med | SQLite writes are <1ms in WAL mode; if measured >5ms add async write task |
| R5 | LSD-3 recovery walker blocks startup if log is huge | Low | Low | Vacuum (LSD-4) keeps log bounded; recovery walker has its own bound in query (LIMIT 100 unresolved) |
| R6 | CPD-4 per-saga correlation breaks F.5/F.6 if concurrent same-kind sagas were *implicitly* relying on evict-and-replace's "first-event-wins" semantics | Med | Med | Property test in CPD-4 must cover concurrent-promote case before merge; reagent + codex must both APPROVE |
| R7 | Host-side LRU (CPD-5) thrashes under high saga rate | Very low | Low | LRU bound 256; saga rate is human-driven (window opens/closes) so we won't exceed |
| R8 | Bot oscillation on saga lifecycle semantics (cf. #637 rounds 5-7) | Med | Low | Apply meta-rule: P1/P2 oscillating ≥3 rounds → document in PR body and merge if reagent APPROVES |
| R9 | Local working tree mods (~700 LOC orphaned launcher edits from before worktree migration) interfere with a batch agent's local cargo build | Low | High | Have batch 1 first agent run `git stash` + verify cargo check on clean tree before starting |

---

## 6. Per-PR acceptance criteria (machine-checkable)

Every PR is "done" when:

1. ✅ `cargo check --workspace` clean
2. ✅ `cargo test -p agentmux-launcher` pass (and `-p agentmux-srv` and `-p agentmux-cef` for cross-cutting changes)
3. ✅ reagent APPROVED on current head
4. ✅ codex returns "no major issues" OR codex absent past 10min from retrigger AND 2nd-poll-90s confirms still absent
5. ✅ No fresh inline P1 from codex
6. ✅ `bump verify` clean (no version drift)

PR-specific exit gates (in addition to the universal):

- **CPD-3:** integration test in `agentmux-launcher/src/saga/integration_tests.rs` named `test_host_crash_during_saga_dispatch` passes
- **LSD-3:** integration test `test_unresolved_saga_marked_failed_compensation_on_restart` passes
- **CPD-4:** `test_concurrent_pool_promotes_correlate_by_saga_id` passes (this is the F.7-style proptest case the evict-and-replace workaround was masking)
- **CPD-5:** `test_duplicate_saga_command_idempotent` (host LRU) passes

---

## 7. End-to-end acceptance (after all 9 PRs)

Smoke test, manual:

1. Open AgentMux. `~/.agentmux/launcher-sagas.db` exists, schema correct.
2. Open + close 5 windows rapidly. Watch `~/.agentmux/log/launcher.log`:
   - 5x `WindowCleanupCascade` sagas fire
   - Each terminates `Completed` (single bracket per saga, no eviction-induced `Failed/Completed` pairs)
3. `agentmux --diag sagas` shows recent sagas with step rows visible.
4. Kill -9 launcher mid-saga (during a slow window close):
   - Restart launcher.
   - `--diag sagas` shows the in-flight saga as `failed_compensation`, with its step rows preserved.
5. Force a host crash (`taskkill /f /im agentmux-host.exe`):
   - Launcher's host pipe drops; saga terminates `SagaActionFailed { reason: "host pipe send failed" }`.
   - On host respawn, new sagas work normally.
6. No `#[allow(dead_code)]` markers on `PipeTarget::Host`, `SagaAction::Failed`, `pool_respawn::promoted_label`, or `SagaCtx::saga_id` (Phase F.7 explicitly retained these as "reserved for future cross-process sagas" — that future is now).

If all 6 pass, mark Thread 3 done in `docs/retro/phase-fg-status-2026-05-01.md`. End of saga reducer architecture migration.

---

## 8. Rollback strategy

If something goes wrong in production after a PR ships:

- **CPD-1, CPD-2, LSD-1, LSD-4** (foundation/passive): revert the PR, ship a patch release. No state migration needed (LSD-1 leaves `launcher-sagas.db` in place but inert).
- **LSD-2** (writes log but unread): same — revert. Log file becomes orphaned but harmless.
- **CPD-3** (wire goes live): revert moves us back to saga-as-narrator. F.5/F.6 still work (existing implicit code path). Revert is safe.
- **CPD-4** (correlation replaces evict-and-replace): revert reinstates evict-and-replace. Concurrent-saga semantics regress to "occasional false-positive SagaFailed brackets" but not a correctness bug.
- **CPD-5** (host LRU): revert means duplicate commands re-execute on host. That can cause visible glitches (window double-reaped). Pair revert with a hot-fix that disables CPD-3's retry-on-pipe-error to avoid resends.
- **LSD-3** (recovery walker): revert means unresolved sagas accumulate forever and `--diag` doesn't show them. Annoying but not breaking.

Each rollback is a single PR revert; no multi-step migrations.

---

## 9. Autonomous execution brief (for the PR sequencer)

When the user says "go", the sequencer should:

1. **Pre-flight:**
   - `git checkout main && git pull origin main`
   - Verify clean working tree (`git status` shows nothing). If dirty → ask user before proceeding.
   - Run `cargo check --workspace` baseline. If fails → STOP, report.
   - Read all 3 specs into the agent context.

2. **Batch 1** (parallel, 3 agents):
   - Spawn 3 background agents per the briefs in §2 batch 1.
   - Each agent uses `isolation: "worktree"` and `run_in_background: true`.
   - Wait for all 3 PR-open notifications.
   - Poll each PR; address P1/P2 per established rules; merge when green.
   - Sync main between merges.

3. **Batch 2** (parallel, 2 agents) — only after all 3 batch-1 PRs merge:
   - Spawn 2 agents per §2 batch 2 briefs.
   - If merge conflict appears between CPD-3 and LSD-2 (R2), pause, message user, ask which to land first.

4. **Batch 3** (parallel, 3 agents) — only after all batch-2 PRs merge:
   - Spawn 3 agents per §2 batch 3 briefs (CPD-4, CPD-5, LSD-3).
   - Land LSD-4 (the trivial vacuum) as a 4th agent in this batch or as a follow-up.

5. **End-to-end smoke** — only after all 9 PRs merge:
   - Run `task package` to build a portable.
   - Notify user with the §7 checklist for manual verification.
   - Update `docs/retro/phase-fg-status-2026-05-01.md` to mark Thread 3 closed.
   - Stop the loop.

---

## 10. Cross-references

- Architecture target: [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md)
- Cross-process dispatch: [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md)
- Launcher saga durability: [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md)
- Phase F+G roadmap (lessons learned + bot oscillation pattern): [`../retro/phase-fg-roadmap-2026-05-01.md`](../retro/phase-fg-roadmap-2026-05-01.md)
- End-of-batch-2 status: [`../retro/phase-fg-status-2026-05-01.md`](../retro/phase-fg-status-2026-05-01.md)

---

End of execution plan.
