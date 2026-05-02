# Saga Reducer Architecture Migration — COMPLETE

**Date:** 2026-05-02
**Status:** All 9 PRs merged. End-to-end smoke test pending.

---

## What shipped

The saga reducer architecture migration (Thread 3 of Phase F+G) is in main. The launcher now drives host-side saga actions over a real wire, with per-saga timeouts, durable lifecycle logging, crash recovery, and operator visibility.

### Merged PRs (chronological)

**Foundations (Batch 1):**
| PR | Title | Merged |
|----|-------|--------|
| [#641](https://github.com/agentmuxai/agentmux/pull/641) | feat(lsd-1): launcher saga log + API | 2026-05-01 19:59Z |
| [#642](https://github.com/agentmuxai/agentmux/pull/642) | feat(cpd-2): host pipe wrapper with reconnect + pending buffer | 2026-05-01 21:43Z |
| [#643](https://github.com/agentmuxai/agentmux/pull/643) | feat(cpd-1): saga_id schema for host-bound Commands | 2026-05-01 20:26Z |

**Integration (Batch 2):**
| PR | Title | Merged |
|----|-------|--------|
| [#644](https://github.com/agentmuxai/agentmux/pull/644) | feat(cpd-3): wire saga dispatch — F.5 + F.6 stop being narrators | 2026-05-01 23:41Z |
| [#645](https://github.com/agentmuxai/agentmux/pull/645) | feat(lsd-2): coordinator integration — saga lifecycle writes to LauncherSagaLog | 2026-05-01 22:49Z |

**Refinements (Batch 3):**
| PR | Title | Merged |
|----|-------|--------|
| [#646](https://github.com/agentmuxai/agentmux/pull/646) | feat(lsd-4): startup retention vacuum | 2026-05-02 00:07Z |
| [#647](https://github.com/agentmuxai/agentmux/pull/647) | feat(lsd-3): recovery walker + --diag sagas | 2026-05-02 00:48Z |
| [#648](https://github.com/agentmuxai/agentmux/pull/648) | feat(cpd-4): per-saga event correlation | 2026-05-02 00:30Z |
| [#649](https://github.com/agentmuxai/agentmux/pull/649) | feat(cpd-5): host-side saga_id LRU + HostFrame parser | 2026-05-02 00:21Z |

**Total:** 9 PRs, ~3,500 LOC, 5 hours wall-clock.

---

## What's now live in main

**Cross-process dispatch (CPD-1 → CPD-5):**
- Launcher → host pipe carries `HostFrame::Command` with mandatory `saga_id`.
- Saga coordinator dispatches `IssueCmd::Host` actions through `HostPipe::send_command()` (no longer log-only narrator).
- Per-saga `Saga::timeout()` (default 5s, F.6 overrides 30s) fires `SagaFailed` if a host action stays unresolved past its budget.
- `claim_terminal()` atomic guard ensures exactly one terminal event per saga across the {Done, Failed, host-send-error, timeout, eviction, SagaActionFailed-listener} race surfaces.
- `HostPipe::cancel_saga(saga_id)` purges pending buffer on saga terminal (no orphan side effects after bracket close).
- Per-saga event correlation by `saga_id` retires the evict-and-replace workaround.
- Host LRU (256 entries) makes saga commands idempotent under retry/replay.
- `host_session_id` generation on `set_writer` prevents stale fanouts from writing to replacement writers.
- Atomic session+writer fetch + `Arc::ptr_eq` guard on write-failure path.

**Launcher saga durability (LSD-1 → LSD-4):**
- SQLite `~/.agentmux/launcher-sagas.db` with WAL mode, FK enforcement, 5s busy timeout.
- Per-step durability: `start_step` → `finish_step` / `fail_step` rows for every coordinator dispatch.
- `next_saga_id` seeded from `max_saga_id() + 1` on startup; `with_log()` returns `Result` and main.rs aborts on seed failure (codex P1 #645 round 2).
- Recovery walker on launcher startup marks unresolved sagas `failed_compensation`; failure_reason is preserved + appended (not overwritten) so original cause survives crash-recovery.
- `--diag sagas` command shows recent sagas with step rows for `failed_compensation` (operator triage flow). Runs BEFORE CEF runtime check — works even when launcher won't start.
- `LauncherSagaLog::open_read_only` for diag invocations; doesn't mutate a log a running launcher owns.
- Startup retention vacuum (default 7-day cutoff, configurable via `~/.agentmux/config.toml` `[saga.launcher] retention_days`); never touches in-flight sagas.

---

## Lessons from this batch

These are new lessons relative to the existing Phase F+G roadmap doc.

### Bot oscillation — much deeper this time

PR #644 (CPD-3) hit **8 review rounds** before merging. Each round addressed real issues:

1. claim_terminal added to apply_action Done/Failed paths
2. Insert-into-in_flight before saga.start() (immediate-completion sagas)
3. claim_terminal in eviction + timeout
4. Keep-saga-in-flight on host-send-error (avoid double-emit)
5. saga_id_of returns real ids
6. (Various stale doc cleanups)
7. Revert host-send-error from terminal back to keep-in-flight (regression from rebase)
8. cancel_saga on claim_terminal (purge pending buffer on terminal)

Each round genuinely fixed a real concurrency edge case. The pattern: codex + reagent each found edge cases the OTHER hadn't seen, and fixing one created another. Eventually converged when `claim_terminal` + `cancel_saga` together cut off both bus and wire paths atomically.

**Meta-rule application:** even at round 8, every fix was real, not oscillation. The meta-rule "≥5 rounds → document and merge" was relaxed because each fix added genuine correctness. This is rare; future heuristic — distinguish "oscillating between equally valid designs" (apply meta-rule) from "incrementally finding new bugs" (keep iterating).

### Workflow hazards encountered

Saved as feedback memories:

1. **`bump --commit` only commits version files.** Code fixes must be staged + committed FIRST. Multiple times across this session, `bump --commit` left in-progress fixes unstaged → PR pushed without the fix → wasted bot-review rounds. Fix: always `git status` after `bump --commit`. Memory: `feedback_bump_commit_only_versions.md`.

2. **`git push origin <remote-name>` is unsafe when local branch ≠ remote name.** When pushing from a worktree where local branch is `agent-foo` but remote is `agenta/cpd-3`, `git push origin agenta/cpd-3 --force-with-lease` resolves to pushing the wrong commit, which can reset the remote to `main`'s HEAD. PR #644 auto-closed once this happened. Fix: always use `local:remote` ref-spec form. Memory: `feedback_push_local_to_remote_branch.md`.

### Parallel PR mechanics

Three patterns showed up:

1. **Same-version conflict.** Multiple parallel agents bump to the same version. Second to merge needs rebase + re-bump. Skip obsolete bump+lockfile commits during rebase, re-bump at end.

2. **Dependency order matters for merging.** CPD-4's saga-id filter required CPD-5's host code to populate `saga_id` on Reports. Codex flagged this as P1 ("accept untagged events") on CPD-4. Resolution: merge CPD-5 first, then rebase CPD-4 — P1 becomes moot.

3. **3-way conflicts on shared files.** CPD-3 + LSD-2 both touched `saga/mod.rs::apply_action`. Initial rebase produced 7 nested conflicts; better path was "spawn a fresh agent to re-derive CPD-3 against current main with LSD-2 integrated, single squashed commit." Hand-merging conflicts deeper than 3-4 regions tends to cost more than re-deriving cleanly.

### Documentation drift

Stale doc comments showed up across **6 separate rounds** as P2 findings. Pattern:
- Original doc: "F.5 IssueCmd is logged-only"
- After CPD-3: doc still said "logged-only" → reagent flags P2
- After fix: doc says "live via HostPipe" but next change to nearby code creates new mismatch

Suggestion: when removing `#[allow(dead_code)]` markers or rewiring comments referencing future PRs, do a `grep -rn "wired in PR LSD-3"` sweep to catch all the references. Local edits leave invisible-to-the-author drift.

---

## Pending work

### Smoke test (manual, ~15 min)

The auto-spawned PR sequencer kicked off `task package` — a portable build for v0.33.578+ should be on the desktop within ~10 min after this doc is written.

§7 manual smoke test (from execution plan):

1. Open AgentMux. `~/.agentmux/launcher-sagas.db` exists, schema correct.
2. Open + close 5 windows rapidly. Watch `~/.agentmux/log/launcher.log`:
   - 5x `WindowCleanupCascade` sagas fire
   - Each terminates `Completed` (no eviction-induced Failed/Completed pairs)
3. `agentmux --diag sagas` shows recent sagas with step rows visible.
4. Kill -9 launcher mid-saga (during a slow window close):
   - Restart launcher.
   - `--diag sagas` shows the in-flight saga as `failed_compensation`, step rows preserved.
5. Force a host crash (`taskkill /f /im agentmux-host.exe`):
   - Launcher's host pipe drops; saga terminates `SagaActionFailed`.
   - On host respawn, new sagas work normally.
6. No `#[allow(dead_code)]` markers remain on `PipeTarget::Host`, `SagaAction::Failed`, `pool_respawn::promoted_label`, or `SagaCtx::saga_id`.

If all 6 pass: Thread 3 is done. The saga reducer architecture migration is operational.

### Known limitations (defense-in-depth, not blockers)

These remain as known edge cases. Saga timeouts + `claim_terminal` + `cancel_saga` provide safety nets.

1. **Multiple host connections during replacement.** If two host clients register concurrently, only one writer is active; old connection's teardown clears the global writer. Production has only one host process — this is defense-in-depth, not a normal-flow bug.

2. **Replacement-race rebuffer overshoot.** When `send_frame`'s `Arc::ptr_eq` mismatch triggers immediate drain, multiple concurrent failed writes can briefly push `pending_buffer` past `PENDING_BUFFER_CAP`. Subsequent normal sends re-enforce cap. Bounded by O(N concurrent writers).

3. **30s host-disconnect window.** Saga commands buffered during host-down get dropped at 30s with `SagaFailed`. Tunable via `DISCONNECT_TIMEOUT` constant if production runs into it.

4. **Retention-vacuum can skip long-running sagas.** A saga in `running` state for 30+ days is never vacuumed (intentional — masking crashes is worse than DB growth). If this becomes a real problem, add a separate "stale-running" check.

---

## Reference

- Architecture target: `docs/specs/SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`
- CPD spec: `docs/specs/SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`
- LSD spec: `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`
- Execution plan: `docs/specs/SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md`
- Phase F+G batch-2 status: `docs/retro/phase-fg-status-2026-05-01.md`
- Phase F+G roadmap: `docs/retro/phase-fg-roadmap-2026-05-01.md`

---

End of saga reducer architecture migration retro.
