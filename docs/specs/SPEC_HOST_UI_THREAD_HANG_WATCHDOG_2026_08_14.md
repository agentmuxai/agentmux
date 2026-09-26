# SPEC: Host UI-thread hang detection, forensic dump, and auto-recovery

**Status:** proposed — not built as of 2026-09-26. Written 2026-08-14 and checked in on 2026-09-26 unchanged apart from this header; re-verify "Current state" below before implementing.
**Motivated by:** incident on 2026-08-14 — main portable window (host process,
`agentmux-0.55.6.exe`) deadlocked for ~16 minutes with zero CPU usage while
one pane/window remained open. The postmortem was written in the investigating
agent's workspace and is not in this repo. In short: the host's UI thread
stopped pumping messages. srv and all agents stayed up. `taskkill` on the host
recovered it: the launcher relaunched the window and it reattached with no lost
sessions. Nothing captured a stack or dump first, so the deadlock's cause is
unknown.

## Problem

The host process's UI thread can wedge (deadlock) while at least one window
is still open. Today nothing in this codebase detects that condition and
acts on it — the operator has to notice Windows' own "Not Responding" UI and
manually kill the process. When that happens:

- No forensic evidence (stack trace / minidump) is captured before the
  process is gone, so the actual deadlock can't be root-caused after the
  fact.
- The backend (`agentmux-srv`) keeps trying to push events to the wedged
  client's WebSocket connection, silently dropping them forever with no
  escalation — including the periodic tick that drives the status-bar time
  pill, which is how the incident became visible at all (an accidental
  side effect, not a designed signal).

## Current state (as of `main` @ `dbd2c9c5`)

This is not a greenfield problem — related infrastructure already exists,
just not wired to cover this case:

1. **UI-thread liveness probing exists, but is only consumed by the
   quit-watchdog, not general hang detection.**
   `agentmux-launcher/src/ui_liveness.rs` — the launcher's supervisor sends
   `Command::ProbeUiThread{nonce}` over the host IPC pipe every 60s
   (`supervisor/windows.rs`, `ui_probe_interval`). The host only replies if a
   posted CEF UI task actually gets scheduled
   (`agentmux-cef/src/ui_tasks/window.rs:1946`,
   `ProbeUiThreadReplyTask`; `launcher_ipc/reporters.rs:206-215`) — silence
   is the hang signal, tracked via `consecutive_misses()`
   (`ui_liveness.rs:106-108`).
   The only consumer of misses is `teardown_backstop.rs`, whose
   `should_teardown()` (L86-99) is gated on being **armed**, and per its own
   doc (L30-34) arming only happens on `PoolDrained`/`OrphanInstance` — i.e.
   **zero user windows already open**. A hang with a window still visible
   (exactly this incident) never arms the backstop, so nothing downstream of
   the probe ever fires.

2. **`agentmux-srv` already has a working hang→recycle path; the host does
   not.** `docs/specs/SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03.md` +
   `srv_liveness.rs`: probes srv's HTTP health endpoint every 10s, and after
   `SRV_HANG_REQUIRED_MISSES=3` (~30s) force-kills and lets the existing
   crash-recycle path respawn it. This spec brings the equivalent pattern to
   the host process, reusing what already exists for srv as the template.

3. **`141222a0` (tabs/panes restore on relaunch) does not cover crash/hang
   recovery.** It snapshots workspace/tab/layout state into
   `Client.meta["session:last_topology"]` during a *graceful* window close
   (before the normal destroy cascade), and only replays it from the
   frontend's cold-start path (`restoreIfAvailable`, `app-init.ts`). A
   hard-killed or deadlocked process never reaches the graceful-close code
   path, so it never wrote that snapshot. **In our incident, the reason no
   work was lost is unrelated to this feature** — it's because session state
   already lives entirely in `agentmux-srv` (which never went down), and the
   relaunched host simply reattached to already-running controllers
   (`"existing controller — skipping spawn"` in the backend log for every
   pre-existing block ID). This is worth confirming/documenting explicitly
   (action item below) rather than relying on it having been observed once.

4. **`agentmux_srv::backend::eventbus`** (`agentmux-srv/src/backend/eventbus.rs`):
   two bounded per-connection `mpsc` channels — `PRIORITY_LANE_CAPACITY = 8192`
   (interactive/terminal) and `BACKGROUND_LANE_CAPACITY = 256` (droppable
   telemetry, e.g. the status-pill tick). `try_send_lane()` (L188-205) is a
   non-blocking `try_send`; on `Full` it logs the WARN we saw
   ("ws egress lane full ... dropping event", L196-199) and drops — nothing
   else. No threshold, no escalation, no proactive disconnect. The
   connection stays registered and keeps silently dropping indefinitely.

5. **Crash-dump-on-hang does not exist anywhere in this codebase.**
   `agentmux-srv/src/crash_monitor.rs` wires `crash-handler` (VEH) +
   `minidumper` for **srv only**, writing dumps to
   `C:\CrashDumps\agentmuxsrv\` — but only in response to an actual
   exception (access violation, heap corruption, aborting panic), not a
   hang, and not `__fastfail` (relies on WER LocalDumps for that). Nothing
   in `agentmux-cef` (the host) references `minidumper`/`crash_handler`/
   `MiniDumpWriteDump` at all. Note this matters mechanically: a deadlock
   produces **no exception**, so the existing VEH-triggered approach
   couldn't catch this class of bug even if it were wired to the host. A
   hang dump has to be taken *externally*, by another process calling
   `MiniDumpWriteDump` against the target PID — which is exactly what tools
   like Sysinternals `procdump -h` do, and works precisely because it
   doesn't require the target's own threads to run.

## Proposal

### 1. Host hang detector (launcher-side, modeled on `srv_liveness.rs`)

Add a second consumer of `ui_liveness`'s probe results, independent of
`teardown_backstop`'s zero-window gate:

- Tighten the probe interval used for hang detection specifically (the
  existing 60s cadence is fine for the quit-watchdog's purposes but far too
  slow here) — align with srv's pattern: probe every 10s, declare a wedge
  after `HOST_HANG_REQUIRED_MISSES = 3` consecutive misses (~30s), instead
  of the ~16 minutes it took a human to notice and act in this incident.
- On wedge declared (window still open, not the zero-window
  `teardown_backstop` case): proceed to steps 2 and 3 below, then force-kill
  and let the launcher's existing supervisor respawn the host — the same
  effect the manual `taskkill` had in this incident, just automatic and
  fast.

### 2. External forensic dump before kill

Before terminating a wedged host, have the **launcher** (which already
tracks the host's PID via the supervisor) call `MiniDumpWriteDump`
externally against that PID — not via `crash_monitor`'s exception-handler
path (wrong mechanism for a hang, per point 5 above), but a direct external
dump, the same technique `procdump -h` uses. Write to a distinctly-named
directory (e.g. `%LOCALAPPDATA%\CrashDumps\agentmux-host-hang\`) so these
are triaged separately from real crash dumps — a hang dump and a crash dump
mean different things to whoever investigates next.

### 3. Escalate the eventbus warning instead of dropping forever

In `try_send_lane()`, track consecutive `Full` results per connection. After
a threshold (e.g. 30 consecutive drops, ~30s at the observed 1/sec tick
rate) log at ERROR with a distinct message (`consumer presumed dead`, not
just `stalled`) and proactively unregister/close that WebSocket connection.
This gives the backend-side signal independent of the host-side probe —
useful if the host-side watchdog above is ever disabled/misses a case, and
makes "genuinely stuck" distinguishable from "briefly slow" in logs without
requiring a human to correlate a modal dialog with a WARN spam.

### 4. Per-instance log tagging

Add PID (or instance/channel ID) to every line in the shared
`agentmuxsrv-v{version}.log.{date}` file. Today only an OS thread ID is
present, which is not unique across the multiple `agentmux-srv` processes
that can share one log file (confirmed during this incident's investigation
— had to cross-reference `netstat`/`Get-NetTCPConnection` output against
log content to prove which `conn` IDs belonged to which running process).

### 5. Document the launcher's relaunch-and-reattach behavior

Write down (spec or code comment) that killing the host process is safe by
design: session/agent state lives in `agentmux-srv`, which the launcher
never touches when only the host is killed, and a relaunched host reattaches
to already-running controllers rather than respawning them. This incident
relied on that behavior working; it should be an explicit, tested guarantee
rather than something an operator discovers by accident during a live
incident.

## Non-goals

- Not attempting to catch or fix the specific deadlock that caused this
  incident — root cause is unknown without a stack trace, which is
  precisely what item 2 above is for. This spec is about detection,
  evidence capture, and recovery time, not about a specific bug fix.
- Not changing `teardown_backstop`'s existing zero-window arming semantics
  — that mechanism is for a different problem (stuck-quit with no windows)
  and works correctly for it today.

## Open questions

- Should `HOST_HANG_REQUIRED_MISSES`/interval be configurable, matching how
  `SRV_HANG_REQUIRED_MISSES` is presumably configured for srv? Check
  `SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03.md` for the precedent
  before inventing a new knob.
- Multi-window hosts: if a host owns more than one OS window, does a wedge
  on the shared UI thread always mean *all* windows are affected? (Almost
  certainly yes, since it's one message pump — worth confirming rather than
  assuming.)
- Dump size/PII: minidumps of the host process may include in-memory
  session content (pane text, etc.). Confirm the dump directory's retention/
  access policy matches whatever already governs srv's crash dumps before
  shipping this.
