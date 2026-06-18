# Retro: Agent Watchdog + Runtime Badge

_Branch: fix/agent-watchdog-and-badge — 2026-03-26_

---

## Why this work was needed

The March 23 CPU spike investigation identified six failure modes. PR #219 fixed the four
most acute ones (FM-1/2/3/5). Two were deferred:

- **FM-4 (no max-runtime watchdog)**: An agent process like `claude --dangerously-skip-permissions`
  can run for days with no automatic termination. On 2026-03-26 we observed PID 95308 doing
  exactly this — started March 16, still running 10 days later at 16–45% CPU. Nothing in the
  system would ever kill it without user intervention.

- **FM-6 (stale agent badge)**: No visual signal in the UI that an agent has been running for
  a long time. Users have no way to notice a runaway agent without checking Activity Monitor.
  The badge gives them a passive signal so they can act before CPU becomes a problem.

---

## What was changed and why each piece was necessary

### `ShellControllerInner` — new fields

Three fields added to the inner struct:

| Field | Purpose |
|-------|---------|
| `spawn_ts_ms: Option<i64>` | Unix ms timestamp at spawn; needed by watchdog (elapsed calc) and frontend badge |
| `last_pty_output: Option<Instant>` | Monotonic instant of last PTY read; needed by idle-output watchdog condition |
| `is_agent_pane: bool` | Whether cmd is an agent CLI; limits watchdog and badge to agent panes only |

All three are `None`/`false` until first spawn so existing tests are unaffected.

### Agent detection heuristic

At spawn time, a pane is flagged as agent if:
- `AGENTMUX_AGENT_ID` env var is set (explicit jekt registration), OR
- `cmd_str` contains `"claude"`, `"codex"`, or `"gemini"` (case-insensitive)

This is intentionally heuristic. False negatives are acceptable (watchdog just doesn't
fire); false positives are low-risk (worst case: an interactive shell is killed after 8h,
but only if the user explicitly sets `term:agentmaxruntimehours`).

### PTY read task — `last_pty_output` update

The `spawn_blocking` PTY read task now holds a clone of `inner: Arc<Mutex<...>>` and sets
`last_pty_output = Some(Instant::now())` on every successful read. The Mutex lock is
grabbed-and-released per-read so it doesn't block PTY I/O.

### `BlockControllerRuntimeStatus` — two new fields

`spawn_ts_ms` and `is_agent_pane` are added with `skip_serializing_if` so they are absent
from JSON when unused (no wire-format bloat for non-agent panes).

### `watchdog.rs` — new module

Runs every 60 seconds. For each running agent pane, checks two independent conditions:

**Condition A — max runtime** (`term:agentmaxruntimehours`, default 0 = disabled):
Computes `(now_ms - spawn_ts_ms) / 1000` and compares to the configured limit.
Kills via `ctrl.stop(true, STATUS_DONE)` which triggers Fix 2's SIGTERM+SIGKILL path.

**Condition B — idle output** (`term:agentidletimeoutmins`, default 0 = disabled):
Downcasts `Arc<dyn Controller>` to `ShellController` via the existing `as_any()` method
(already on the trait) and calls `last_output_secs_ago()`. Kills on same path as A.

Both limits default to 0 (disabled). The watchdog loop skips the entire scan when both
are zero, so there is zero overhead when not configured.

### `wconfig.rs` — two new settings

`term:agentmaxruntimehours` and `term:agentidletimeoutmins` follow the existing
`skip_serializing_if = "is_zero_f64"` pattern (absent from JSON when 0).

### Frontend badge (`agentRuntimeLabel` memo + `AgentRuntimeBadge` overlay)

The memo returns `null` unless:
- `is_agent_pane = true`
- `shellprocstatus = "running"`
- `spawn_ts_ms` is set
- elapsed time ≥ 1 hour

When all conditions are met it returns `"Xh Ym"`. The badge renders as an absolute-
positioned overlay in the top-right of the term content area (same layer as TermStickers),
so no changes to blockframe.tsx or the shared header framework were needed.

---

## What was deliberately NOT done

- **No default limits set**: Both watchdog conditions require explicit opt-in. Automatically
  killing a user's agent after 8h would be surprising and destructive. Users who want limits
  set them in `settings.json`.

- **No kill confirmation dialog (Fix 6 stretch goal)**: The fix plan described an interactive
  dialog. Deferred — the badge gives the visual signal; the watchdog gives the automated
  kill. A dialog adds complexity for a case that the user can handle by just closing the pane.

- **No idle timeout for interactive shells**: `is_agent_pane = false` for plain shells.
  Idle timeout on interactive terminals would be user-hostile.
