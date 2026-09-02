# Retro — Agent1 (stable channel) showed ambiguous "Waiting…/Working…" for ~12h while a user-launched long-running dev process was attached (2026-07-16)

## Correction notice

An earlier draft of this retro concluded the agent was **genuinely idle and
the UI was mislabeling it**, and proposed a frontend fix (seed `lastEventMs`
from backend `session:last_activity_ms`, force-recover to `Idle` faster). That
conclusion was **wrong**, and that fix would have been actively harmful — it
would have painted a green "Idle" on an agent that legitimately had a
long-running process attached. The corrected analysis below supersedes it.
The mistake is itself a lesson (see Lessons): "process quiet" was read as
"turn abandoned," when the truth was "agent deliberately running a long-lived
background process."

## TL;DR

Agent1 (host/persistent mode, `stable` channel, v0.53.6) displayed an
ambiguous "Waiting…/Working…" state for ~12 hours (2026-07-15 20:54 →
2026-07-16 08:59). It was **not** hung and **not** cosmetically mislabeled: it
had genuinely started a long-running background process — its own dev stack
via `./scripts/dev-agent.cmd` (which runs `task dev`: vite + cargo-watch + a
launched nested AgentMux instance / CEF) — launched with the Bash tool's
`run_in_background: true`. The status UI collapsed "a user-launched
long-running process is attached and alive" into the same generic
"Waiting…/Working…" affordance it uses for an active generation or a hung
turn, giving no way to tell them apart. When the user killed the dev stack at
08:59, the state cleared and Agent1 immediately resumed (and launched a fresh
`dev-agent.cmd`). The gap is a **missing distinct status for "long-running
attached process,"** not a broken watchdog or a mislabel.

## Evidence

Live block meta (`db_block` oid `c830ecec-…`, from the channel's *actual*
live data dir `…\channels\stable\versions\0.53.6\data\db\objects.db` — note
NOT the stale top-level `…\channels\stable\data`, which was four weeks old;
always resolve the live dir from the running srv process's `--wavedata` arg):

- `session:active_pid: 70416` — a live `claude.exe`. Sampled CPU twice, 5s
  apart: **0.0 delta** → the CLI itself was idle, not busy-looping.
- `session:last_activity_ms` frozen at **2026-07-15T20:54:20Z** during the
  hang; on re-check after the user killed the process it had jumped to a fresh
  timestamp and `session:line_count` had grown 19482 → 19682, with
  `term:ambient_summary` updated to "Merged Armory UI, fixed session idle
  problem" — i.e. it resumed and did real work the moment the dev stack died.

Transcript (`…/projects/…agent1-06309/76f11793-….jsonl`):

- The last entry before the silence is a **clean, complete assistant text
  message at 20:54:20** ("Fixed the P1…pushed…"). There is **no pending
  foreground tool call** after it — the CLI genuinely finished its turn and
  went idle waiting for input.
- Scanning all Bash tool calls for `tool_use` with no matching `tool_result`,
  and for the dev stack: the agent ran `./scripts/dev-agent.cmd` many times
  over several days, each with `run_in_background: true`. The relevant one:

  ```bash
  # 2026-07-15T20:24:05Z, run_in_background: true
  cd "…\agent1-06309\agentmux" && (
    while true; do sleep 120; echo "[heartbeat] dev still alive $(date +%H:%M:%S)"; done &
    HEARTBEAT_PID=$!
    trap "kill $HEARTBEAT_PID 2>/dev/null" EXIT
    ./scripts/dev-agent.cmd TITLE="armory: refine responsive layout"
  )
  ```

  This launched at 20:24 and its dev stack stayed alive through the whole
  window. Immediately after the user killed it at 08:59, the transcript shows
  a **new** identical `dev-agent.cmd` (`2026-07-16T08:59:02Z`,
  `run_in_background: true`, still pending) — direct confirmation that killing
  the old dev stack is what unblocked the agent, which then relaunched one.

## Root cause

The agent-pane status model (`frontend/app/store/agent-pane-state/`, the
`TurnPhase` discriminated union: Idle / Submitting / Streaming / Interrupting
/ Done / Disconnected) has **no state representing "a long-running
process/tool the agent launched is attached and alive."** A `run_in_background:
true` Bash task is exactly that: the tool call returns a task handle
immediately (correct), the process keeps running, and the pane surfaces its
liveness as the same undifferentiated "Waiting…/Working…" it shows for an
in-flight model turn. From the user's side there is no way to distinguish:

1. the agent is actively generating a response,
2. the agent is blocked waiting on a provider (rate limit — this one at least
   got a dedicated label in a prior fix),
3. the agent kicked off a long-running dev server / watch process that is
   working as intended and will run until told to stop,
4. the agent's turn genuinely hung.

Cases (3) and (4) look identical, which is precisely why a healthy 12-hour dev
server reads as "stuck." This is an **observability / status-model gap**, not
a control-flow bug.

### Why the liveness watchdog never auto-recovered it

The pane has a watchdog (`StreamWatchdogTick`) that force-recovers a `Streaming`
turn to `Idle` after `LIVENESS_RECOVERY_MS` = **180s** of no stream activity
(`types.ts:707`, unchanged on current `main` v0.53.6+). Agent1's command
deliberately emits a keepalive **every 120s**:

```bash
while true; do sleep 120; echo "[heartbeat] dev still alive $(date +%H:%M:%S)"; done &
```

120s < 180s, so if those heartbeat lines refresh the pane's `lastEventMs` (as
background-task output plausibly does), the 180s silence threshold is **never
reached** and the watchdog never fires — the pane stays "Working" indefinitely
even by the recovery path's own design. (The agent almost certainly added this
heartbeat to defeat the *idle-kill* of its background process — cf.
`agentmux-bashwrap/tests/idle_kill_full_process_tree.rs` — not realizing it
would also pin the pane's busy indicator on.) The exact channel the heartbeat
feeds (main turn stream vs. a separate background-task sub-stream) wasn't
confirmed without a live repro; but note `session:last_activity_ms` (the
backend CLI-stdout clock) stayed frozen at 20:54 while the frontend indicator
stayed lit — consistent with the heartbeat feeding the *frontend* pane stream
but not the *backend* CLI turn clock, i.e. the two liveness signals
disagreeing, which is itself the wiring seam to fix.

### Why this is NOT the two nearby, real, already-fixed bugs

- **Not the bashwrap PTY-EOF hang** (`docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md`,
  fixed by #2156 — `fix(bashwrap): disable git pager, kill idle-hung children
  instead of leaking the wrapper`, merged 2026-07-14, confirmed **present in
  v0.53.6** which Agent1 was running). That bug hangs the *wrapper* on a
  foreground command whose grandchild holds the PTY slave open; #2156 bounds
  the `publisher_handle` wait to 5s. Agent1's dev stack was a *backgrounded*
  task that returned its handle fine — the wrapper didn't hang, the process
  legitimately kept running.
- **Not the liveness-recovery watchdog** (`docs/specs/SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md`,
  shipped #1842). That watchdog force-recovers a *hung* `Streaming` turn to
  `Idle` after 180s. Here the turn wasn't hung — and if the watchdog *had*
  fired, it would have been **wrong**, flipping a legitimately-busy agent to
  Idle. This case shows the watchdog's premise ("quiet Streaming = probably
  hung") doesn't hold when the quiet is a healthy long-running attached
  process.

## Fix direction

Add a distinct, first-class status for "long-running attached process," so the
pane can show e.g. *"Running: dev-agent.cmd (14m)"* instead of a generic
"Waiting…". Sketch, in priority order:

1. **Model it in the reducer.** The pane already tracks background tool tasks
   at some layer (the Bash tool returns a task id and streams output). Thread
   "≥1 live background task" into the pane's status derivation as a state
   *distinct from* the turn `Streaming`/`Idle` phase — a long-running task can
   coexist with an Idle turn (exactly Agent1's situation: idle CLI + live dev
   server). This is the missing axis: turn-phase and attached-process-liveness
   are independent, and the UI currently only renders one.
2. **Surface it in `AgentFooter.tsx`** with its own label + affordance (task
   name, elapsed time, and ideally a stop control), the same way the prior
   "Rate limited — retrying…" label was added as a distinct sub-state rather
   than overloading "Working…".
3. **Don't let the liveness watchdog touch a pane with a live background
   task** — its "quiet = hung" heuristic must exempt panes that have a known
   attached long-running process, or it will mislabel them.

This is the concrete, correct interpretation of the standing "we want a solid
state-machine reducer for status" ask: the reducer core is mature (122 tests,
discriminated union), but it models only the *model-turn* lifecycle. The
flakiness users keep hitting is that a second, independent dimension —
**attached-process liveness** — isn't represented at all, so it leaks into the
turn-phase label and reads as stuck.

## Files

| File | Relevance |
|------|-----------|
| `frontend/app/store/agent-pane-state/reducer.ts` / `types.ts` | Turn-phase state machine — models the model-turn lifecycle only; no attached-process-liveness axis (**primary gap**) |
| `frontend/app/view/agent/components/AgentFooter.tsx` | Renders the "Working…" / "Rate limited…" labels — where a new "Running: <task>" affordance belongs |
| `frontend/app/view/agent/useAgentStream.ts` | Background-task subscription + watchdog dispatch; watchdog must exempt panes with live background tasks |
| `agentmux-srv/src/backend/blockcontroller/session_stats.rs` | `record_line()` → `session:last_activity_ms` — accurate CLI-idle ground truth; froze correctly here (the CLI *was* idle) |
| `docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md` | Adjacent, genuinely-different bug (wrapper hang), already fixed by #2156 — ruled out here |
| `docs/specs/SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md` | Liveness watchdog — its "quiet = hung" premise breaks for long-running attached processes |

## Lessons

1. **"Process quiet" ≠ "turn abandoned."** My first-pass conclusion made
   exactly this error: `claude.exe` at 0% CPU with a frozen activity clock
   looked idle, so I assumed the turn was done and the UI was lying. The CLI
   *was* idle — but the agent had deliberately launched a long-running process
   that was doing precisely what it was told. Idleness of the model process is
   not the same as absence of work.

2. **A status label with fewer states than the system has situations will
   always be flakey.** "Working…" is asked to mean generating, waiting on
   provider, running a long-lived attached process, and hung. Any two of those
   colliding reads as a bug. The fix is more states, not a better watchdog.

3. **Turn-phase and attached-process-liveness are orthogonal.** The reducer
   models one; the recurring "stuck" reports come from the other leaking into
   it. An idle turn with a live dev server is a normal, expr­essible state that
   currently has nowhere to live.

4. **Resolve the live data dir from the running process's `--wavedata`, not
   the channel's top-level `data/`.** The top-level path was a four-week-stale
   pre-version-bump copy; reading it first sent the initial investigation to
   the wrong DB.
