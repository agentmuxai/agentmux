# Retro: subagent-backfill replay storm crashed the launcher via srv OOM (2026-07-17)

**Date:** 2026-07-17
**Severity:** High — the entire app (not just a window) died and stayed dead until manually relaunched.
**Status:** Root-caused; two fixes implemented (this PR + a companion launcher PR).

## 1. What the user saw

The "shared" (portable) v0.53.5 instance went unresponsive and had to be manually relaunched. No in-app error — the whole process tree was gone.

## 2. Timeline (from `~/.agentmux/logs/agentmux-launcher.log` and the srv log)

All times below are 2026-07-17, UTC (log timestamps) with local PDT in parens.

| Time (UTC) | Event |
|---|---|
| 15:06:00–15:06:09 | `agentmux-srv` logs a burst of **~1,030 "subagent spawned" events in ~10s**, all for one logical subagent (slug `quizzical-tumbling-valiant`, parent block `7142b72e-...` — this session's own agent pane), each broadcasting a WebSocket event to every connected window. |
| 15:06:01 (08:06:01) | srv exits, code `-1073740791` (`0xC0000409` — Windows' generic fail-fast/abort code; **not** a literal stack-buffer overrun despite the status name — this is what a Rust allocation failure looks like on Windows). Launcher logs "srv exited UNEXPECTEDLY ... respawning srv + recycling host (restart 1/3)". |
| 15:06:04 | Respawned srv (new PID) crashes again, same code, ~3s after spawn. Restart 2/3. |
| 15:06:07 | Third respawn crashes again, ~3s after spawn. Restart 3/3. |
| 15:06:10 | `srv restart budget exhausted (3 in 120s) — terminating launcher`. **The whole app exits.** |

Every one of the three respawned srv processes crashed within ~3 seconds of starting, and the "subagent spawned" burst (15:06:00–15:06:09) is co-extensive with the entire cascade — each fresh srv immediately re-hit the same trigger on startup.

Corroborating, live, at investigation time: this machine's system commit charge was measured at 87.5/87.7 GB (~99.8%, ~200 MB headroom) — a `tsc --noEmit` run OOM'd twice needing a bumped Node heap, and a bare PowerShell `Get-Counter` call itself threw `System.OutOfMemoryException`. The system has essentially zero commit slack; a sharp allocation spike is enough to tip any process over.

## 3. Root cause: unbounded historical replay on every cold backfill

`agentmux-srv/src/backend/subagent_watcher.rs`'s `scan_session_subagents` → `scan_subagents_dir` runs whenever a pane (re)opens or a block re-registers with a pre-existing session id (`agentmux-srv/src/server/reactive.rs:350`) — including every srv restart, since a fresh process's block controllers re-register on startup. It walks:

- every `agent-*.jsonl` directly under `subagents/`, **plus**
- every `agent-*.jsonl` under **every workflow run directory this session has ever produced**, under `subagents/workflows/<run-id>/`.

There is no bound on directory age or count — it replays the session's **entire lifetime** of subagent activity, every time. `process_jsonl_change`'s `is_new` check (`!session.subagents.contains_key(&agent_id)`, line ~791) is keyed off an in-memory `HashMap` that starts empty on every fresh srv process, so a restart can never tell "already told a client about this" from "genuinely new" — every historical file reads as new again, each one doing a full-file read + JSON parse + `tracing::info!` + WebSocket broadcast to every connected window.

This session's pane has accumulated a large `subagents/` corpus from real Task/Workflow-tool usage over many days of debugging work (including the swarm/subagent-diagnostics work in tasks #43/#44). On a cold restart, replaying that whole corpus is expensive enough — and, on a machine already near its commit ceiling, fast enough — to itself trigger the next OOM, which is why all three respawns died identically within ~3 seconds: each one hit the same full-history replay on startup and re-crashed before finishing it.

This is the same duplicate-subagent-slug class of bug tracked in tasks #43/#44 (`[[agentmux-swarm-duplicate-subagent-groups]]`), but at roughly 100x the previously-observed scale (8+ duplicates → 1,000+) — a cold-restart replay storm, not just a display-dedup glitch.

## 4. Contributing gap: no OOM-aware retry for srv exits

The launcher already has commit-aware "wait for memory to recover" handling for the **CEF host** process (`agentmux-launcher/src/mem_supervisor.rs`'s `classify_host_exit`/`SystemOom`, wired in `agentmux-launcher/src/supervisor/windows.rs`'s host-exit arm) — built for exactly this class of transient system-OOM crash (`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`). But it was only wired to the host-exit branch; the **srv**-exit branch treated every exit identically, burning the fixed, fast `SRV_RESTART_BUDGET` (3 restarts / 120s) regardless of cause. Since srv is a plain Rust process (not Chromium), it never emits `CHROMIUM_OOM_EXIT_CODE` (`0xE0000008`) — but `classify_host_exit`'s low-commit-at-exit-time fallback branch is generic and already catches this case once wired up; it doesn't need a Rust-specific OOM code. Without that wiring, three fast identical crashes (all really the same transient condition) looked indistinguishable from three genuine bugs and burned the budget to zero, killing the launcher instead of waiting out what was likely a recoverable few seconds of commit pressure.

## 5. Fixes

**Fix A (this PR) — bound the cold-backfill replay** (`agentmux-srv/src/backend/subagent_watcher.rs`): `scan_subagents_dir` now caps replay to the `BACKFILL_MAX_FILES` (200) most-recently-modified `agent-*.jsonl` files, by mtime, regardless of how large the session's total historical corpus has grown. Workflow run journals (one small file per run, not per member) are still always processed so `workflow:updated`/run-status telemetry stays accurate even when membership replay is capped. This directly bounds the worst-case cost of every future pane reopen / srv restart to a fixed, small amount of work.

**Fix B (companion launcher PR) — wire srv exits through the existing OOM classifier** (`agentmux-launcher/src/supervisor/windows.rs`): the srv-exit arm now calls `mem_supervisor::classify_host_exit(code, commit_free)` before deciding how to respond. A `SystemOom`-classified exit waits for commit to recover (same backoff/deadline/give-up-dialog machinery already proven for the host arm, on its own `srv_oom_restarts` budget) instead of consuming the fast crash budget; a genuine `Abnormal` exit keeps the existing fast-budget behavior unchanged.

## 6. Scope not covered by these fixes

- **Why the corpus got this large in the first place** — not investigated here. `subagents/` directories are never pruned/archived; a long-lived, heavily-used pane will keep accumulating them. Worth a follow-up if backfill cost (even bounded) becomes noticeable.
- **Unix/macOS srv supervision** (`agentmux-launcher/src/supervisor/unix.rs`) has no retry budget for srv at all currently — any unexpected srv exit terminates the launcher immediately (no OOM classification, no respawn). Out of scope here (this incident was Windows-only); flagged for future parity work.
- **Whether the pre-existing duplicate-slug bug (tasks #43/#44) also has a live, non-restart-triggered trigger** — this incident's storm was specifically a cold-restart replay; whether the same slug can still collide during a single live session wasn't re-verified here.

## 7. Sources

- `~/.agentmux/logs/agentmux-launcher.log` (crash-loop timeline)
- `~/.agentmux/logs/agentmuxsrv-v0.53.5.log.2026-07-17` (subagent-spawn burst)
- `agentmux-srv/src/backend/subagent_watcher.rs` (`scan_session_subagents`, `scan_subagents_dir`, `process_jsonl_change`)
- `agentmux-launcher/src/mem_supervisor.rs`, `agentmux-launcher/src/supervisor/windows.rs`
- `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`, `docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md`
- `[[agentmux-swarm-duplicate-subagent-groups]]`, `[[agentmux-pagefile-oom-crash-crossref]]`, `[[agentmux-memory-commit-vs-virtual]]` (memory)
