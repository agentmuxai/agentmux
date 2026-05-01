# Phase F+G Status — 2026-05-01

Snapshot at end of autonomous PR sequencer session.

Version after merges: **0.33.560**.

## What landed today

### Architecture-completeness sprint (already merged before this session)

| PR | Title | Notes |
|----|-------|-------|
| #629 | step1: process state machine | |
| #630 | step2: tabbar reactive | |
| #631 | step3: workspace reducer | E.4 lazy-import added (since flipped — see #638) |
| #632 | step4: pane reducer | |
| #633 | step5: tear-off saga (PR 1) | force-flag pattern introduced |
| #634 | step6: saga durability (PR 1) | log + replay scaffolding |
| #635 | step7: E.7 integration tests + invariants_hold proptest | template for F.7 |

### Thread 1 — Saga durability completion + F.6 cascade saga

| PR | Title | Result |
|----|-------|--------|
| #636 | saga durability PR 2 (resume + --diag + crash-recovery) | merged 13:44:34Z (7 rounds) |
| #637 | F.6 window-cleanup cascade saga | merged 13:51:48Z (7 rounds) |

#637 ran into bot oscillation between codex and reagent on `classify_run_saga_result`'s default for non-timeout `Err`. Documented as a known limitation; existing srv-side sagas all drive compensation in their inner future, so `Compensated` is the right default for them. Future sagas that abort without compensating must explicitly construct `SagaTerminal::Failed`.

### Thread 2 (Batch 2) — Run in parallel

| PR | Title | Result |
|----|-------|--------|
| #638 | E.4 flip handle_move_tab to strict mode | merged 14:17:06Z (1 round, clean) |
| #639 | step5-pr2 DeleteWorkspace saga | merged 14:37:36Z (2 rounds — version conflict from parallel PR) |
| #640 | F.7 host reducer proptests + cleanup audit | merged 14:52:31Z (1 round, clean) |

`#640` cleanup audit: 8 `#[allow(dead_code)]` retained with refreshed comments (mostly reserved variants per F-spec §4.3), 5 deleted as stale (e.g. `enum-level SagaAction allow`, `emit_failed allow` since F.6 evict-and-replace calls it, `state::State::launcher_start_ms` write-only field).

`#640` proptest count: 8 new tests at 64 cases each — 5 reducer-arm + 3 saga-coordinator. Per-test cap kept low for CI speed per F.7 brief.

## Lessons captured (added to roadmap)

1. **Bot oscillation on contested surfaces.** Codex flipped twice on `classify_run_saga_result` defaults across rounds 5/6/7 of #637. Meta-rule applied: P1/P2 oscillating ≥3 rounds → document the limitation in PR body and merge if reagent is APPROVED on current head. Saved hours.
2. **Saga-as-narrator pattern.** F.5/F.6 sagas log `IssueCmd::Host` events that don't have cross-process dispatch yet — saga records the intent in the durable log even when the host loop can't act on it. Lets crash recovery rebuild a coherent picture without forcing the cross-process dispatch wire to land first.
3. **Concurrent same-kind sagas: evict-and-replace.** Apply when a fresh trigger arrives for an in-flight saga key. Used in F.6 (`window_cleanup` keyed on label).
4. **`force` flag on commands for compensation bypass.** Pattern from #633 (tear-off) — commands carry `force: bool` so the saga can drive cascade ops without re-triggering the very guards (last-tab, workspace-id) the saga is enforcing at a higher level. `#[serde(default)]` for backward compat.
5. **Parallel PRs share `package.json` real estate.** When two agents bump in parallel, the second's PR will fail reagent's "version already in main" check. Resolution: rebase + extra bump. Mechanical but unavoidable until we serialize the bump step.
6. **Codex review propagation lag is 10–60s.** PR-open auto-triggers codex; force-push does NOT. Always retrigger via a5af PAT after force-push and wait at least 10 min before declaring "absent". Force-push relocates inline comments — re-anchor by `commit_id startswith <head8>`.
7. **`@codex review` and `@gemini review` only fire from a5af, not AgentA-asaf.** Use `gh api ... --header "Authorization: token $A5AF_PAT"` to post the trigger comment. (saved in memory)

## What's left

### Thread 3 — Cross-process dispatch (needs spec, then implementation)

Currently sagas in srv emit `IssueCmd::Host` events that go nowhere — they're logged but don't reach the launcher. The cross-process pipe needs a real wire so the saga coordinator can drive launcher-side state transitions transactionally. Scope is non-trivial:

- Pipe protocol design (length-prefixed JSON? CBOR? backpressure semantics?)
- Failure modes: launcher disconnect mid-saga → saga timeout → compensation
- Identity: who owns the pipe (launcher process? srv child?)
- Reconnection on launcher restart (recovery walks unresolved sagas — does it re-issue or fail-them-out?)

**Action:** spec doc first. Recommend `docs/specs/SPEC_CROSS_PROCESS_DISPATCH_2026-05-XX.md`. Pause autonomous work until user reviews spec.

### Misc cleanup

- **Local main working tree has ~700 uncommitted lines of launcher mods** (`agentmux-launcher/src/{ipc,reducer,saga,srv_spawner,state}.rs`). These are orphaned from when F.6 was started in-place before switching to a worktree. The merged #637 has the polished version. Confirm on next session whether to discard or merge into a follow-up. They're tracked in `git status` but never committed.
- **Phase B locked branches in worktrees**: `agenta/saga-durability-pr2-recovery`, `agenta/f6-window-cleanup-cascade-saga`, `agenta/step5-pr2-delete-workspace-saga`, `agenta/e4-move-tab-strict`, `agenta/f7-host-reducer-proptests` — all merged but worktree dirs hold the branch refs. Will prune naturally when worktrees are cleaned up.
- **5 pre-existing test failures** in `agentmux-srv` reproduce on origin/main pre-this-session: providers / reactive / session-archive / filestore / wcore modules. Not regressions. Worth a triage pass but separate effort.

### Soak items / follow-ups (not urgent)

- **E.4 strict mode (#638)**: now strict — if any production logs surface `tab not found in state` errors, they're real bugs the lazy-import was masking. Watch for ~1 week.
- **DeleteWorkspace saga (#639)**: compensation is record-only (delete is destructive, no inverse). Saga log itself is the post-mortem. If a delete fails partway, recovery marks `failed_compensation` for operator review. No automated rollback.
- **Saga durability (#636)**: crash recovery now walks unresolved sagas on startup. If recovery encounters a step whose inverse can't be derived, it pushes to errors (failure, not skip). Watch for these in `~/.agentmux/saga.db` after crashes.

## Roadmap docs

- `docs/retro/phase-fg-roadmap-2026-05-01.md` — pre-session plan
- `docs/retro/reducer-architecture-gaps-2026-05-01.md` — gap analysis
- `docs/retro/next-steps-architecture-completeness-2026-05-01.md` — sprint ordering
- `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — F-series scope
- `docs/specs/SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — E.4 spec
- `docs/specs/SPEC_SAGA_DURABILITY_2026-05-01.md` — saga durability scope
- **this doc** — end-of-session snapshot
