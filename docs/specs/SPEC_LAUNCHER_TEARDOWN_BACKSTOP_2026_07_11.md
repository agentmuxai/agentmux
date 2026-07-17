# SPEC: Launcher-side teardown backstop (UI-thread liveness probe + armed J0 teardown)

**Date:** 2026-07-11
**Status:** Phase 1 merged 2026-07-11 (observe-only probe). Phase 2 implemented 2026-07-16 — armed state machine in `agentmux-launcher/src/teardown_backstop.rs`, arm/disarm hooks in `ipc/server.rs`'s post-reducer event pass (PoolDrained / OrphanInstance arm; WindowOpened disarms; supervisor disarms on every host exit), teardown execution in the supervisor's 5s check tick (`TerminateJobObject(J0)`, launcher exit code 86), `consecutive_misses` counter added to `ui_liveness` (any pump since a probe's send disqualifies it as wedge evidence), and the §Verification `debug:hang_ui` host command (double-gated: `AGENTMUX_DEBUG_HANG=1` + explicit invoke).
**Tracking:** #2092 (carved out of Discussion #1680 §9.4); shared prerequisite with #942 §8
**Scope:** `agentmux-common` (protocol), `agentmux-cef` (probe reply), `agentmux-launcher` (prober + rule)

## Problem

Every quit backstop shipped so far runs **inside the host**: the #2043 quit
watchdog, Stage-2 executors, orphan_reconcile's nothing-will-pump drive — all
of them die with a wedged host. If the host's UI thread hangs (deadlock,
runaway synchronous work) after the last user window closes, nothing above it
notices: the launcher sees a Running child and an open pipe (the host's
tokio IPC reader keeps pumping happily — **pipe traffic is not liveness**),
and the process tree lingers until the user kills it by hand. This is the one
undelivered item from Discussion #1680's §9 scorecard.

## Key insight — the trigger already exists

The launcher already *detects* the arming condition; it just does nothing
host-independent with it:

- `ReportPoolDrainDecision { was_last: true }` — the host's own
  "last user window closed, drain begins" report (Pillar 2).
- The WRR reducer's `OrphanInstance` drift ("Last user-visible window
  closed; host still alive (likely holding warm pool)") + the
  `window_cleanup_cascade` saga — both observed firing in every clean quit's
  launcher log.

After those fire, a healthy host exits within seconds (live-measured: solo
~257ms, multi-window ~2–5s with pool sweep). A host still alive long after
is either legitimately busy (rare, bounded) or wedged. The backstop's job is
to tell those apart — which needs a liveness probe that a wedged host
CANNOT answer.

## Phase 1 — UI-thread liveness probe (observe-only)

New protocol pair (same shape as every existing launcher↔host exchange):

- `Command::ProbeUiThread { nonce: u64 }` — launcher → host, over the
  existing host pipe (same channel the saga `IssueCmd::Host` commands use).
- `Command::ReportUiThreadAlive { nonce: u64 }` — host → launcher, following
  the `Report*` convention.

**The reply MUST round-trip through CEF's UI thread.** The host's pipe
reader is a tokio task; replying from the handler would prove only that the
reader is alive — the exact non-signal this spec exists to avoid. The
handler posts a `wrap_task!` UI task whose `execute()` sends the report; a
wedged (or pre-ready — the known `post_task` silent-drop) UI thread simply
never replies, and that *silence is the signal*. No host-side timer needed.

Launcher side (Phase 1): a low-rate prober (every 60s while the host is
`Running`) records `last_ui_alive: Instant` + logs round-trip latency.
**Observe-only** — no teardown consumer yet, mirroring how Step 4 Phase 1
(GetSnapshot fetch) and every other risky mechanism in this program landed:
protocol + telemetry first, consequence second, so a false-positive bug
can't kill anything while the signal's real-world behavior is baked.

Idempotency/dedup: nonce echo; the launcher treats any `ReportUiThreadAlive`
as "alive as-of receipt" (a late reply to an old nonce still proves the UI
thread pumped after the probe was sent — staleness is bounded by the probe
interval, which is all the rule consumes).

## Phase 2 — the armed teardown rule

State machine in the supervisor (`run_windows`'s select loop), Windows first:

```
Disarmed ──(was_last drain report OR OrphanInstance-with-zero-user-mirror)──▶ Armed(t0)
Armed    ──(host exits)──────────────────────────────────────────────────▶ normal supervised-exit path
Armed    ──(any user window opens per mirror)────────────────────────────▶ Disarmed
Armed    ──(t > t0 + GRACE and probe unanswered for ≥ 2 intervals)────────▶ Teardown
```

- `GRACE = 30s` — an order of magnitude above the slowest healthy quit
  observed (multi-window with pool sweep), far below "user annoyed."
- Teardown = log loudly (`[teardown-backstop] host wedged with zero user
  windows — terminating job`), then `TerminateJobObject(J0)` and launcher
  exit with a distinct code. **I2/I3 hold by construction**: J0 is the
  launcher's own unnamed job; the blast radius is exactly the processes it
  spawned. This is the one deliberate exception to "never kill what a saga
  can reconcile" — there is nothing left to reconcile when zero user windows
  remain and the UI thread is provably dead.
- False-positive guards (each maps to a known legitimate zero-window state):
  - **Startup**: rule disarmed until the first user window registers.
  - **Crash-restart gap**: the supervisor owns restarts — the state machine
    is suspended between abnormal exit and respawned-host readiness (the
    same span the splash-respawn logic already brackets).
  - **Crash-reproject**: reproject re-opens windows within the grace; the
    mirror's `WindowOpened` disarms. A reproject slower than 30s with the
    UI thread also unresponsive IS a wedge.
  - **Probe transport failure** (pipe down): does NOT count as "unanswered"
    — a disconnected pipe already has its own supervision (30s buffer
    budget); only a delivered-but-unanswered probe is UI-thread evidence.

## Verification plan

- Phase 1: unit tests on the protocol arms (nonce echo, dedup) + live: probe
  latency lines visible in the launcher log across a normal session, a
  crash-restart, and a reproject.
- Phase 2: reducer-style unit tests for the state machine (arm, disarm on
  window-open, teardown on expiry); live wedge reproduction via a
  debug-only host IPC command (`debug:hang_ui`, gated behind
  `AGENTMUX_DEBUG_HANG=1`) that parks the UI thread in a sleep — then close
  the last window and watch the backstop tear the tree down within
  GRACE + 2 intervals, with zero processes left (the E2E harness's wmic
  sweep as the assertion).

## Non-goals

- srv supervision (#942 Phase 2) — separate mechanism, same family.
- macOS/Linux parity — the state machine is portable but lands
  Windows-first like every mechanism in this program; unix.rs mirrors after
  the Windows bake.
- Replacing the host-side watchdog (#2043) — that stays as the first line;
  this is the layer beneath it.
