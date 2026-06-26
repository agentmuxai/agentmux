# Incident Report — AgentMux Closed Unexpectedly

**Date:** 2026-06-26  
**Reported by:** User  
**Severity:** Medium (data not lost; agents were mid-run)

---

## What happened

AgentMux (v0.49.1) was left running overnight with two active agent panes. When the user
returned, the window was gone. The app did **not** crash — it was **killed by Windows** due
to critical system-wide page file exhaustion.

---

## Timeline (all times UTC)

| Time (UTC) | Event |
|------------|-------|
| ~23:59 Jun 25 | srv log rotates to new file (Jun 25 → Jun 26); session running normally |
| 00:10–00:38 | Two agents (blocks `50a320f7`, `c44d6df1`) receiving turns, writing blockfiles continuously |
| 00:53:43 | Agent `c44d6df1` health: **Healthy → Stalled** |
| 00:55:13 | Agent `c44d6df1` health: **Stalled → Dead** (sub-process died; likely OOM on the agent CLI itself) |
| 01:15–06:23 | srv alive (session archiver sweeps every hour; no host activity) |
| 06:17:18 | Agent inputs resume on both blocks — `c44d6df1` was restarted and running again |
| 06:28:51 | Frontend: `agentActivity busyCount=1 panes=[c44d6df1]` — agent actively streaming |
| 06:25–06:32 | Host mem heartbeats show **`avail_page_gb` at 0.1 GB** (out of 87.7 GB total — 99.9% committed) |
| 06:31:17 | `agentActivity busyCount=0` — agent turn finished |
| 06:32:39 | `agentActivity busyCount=1` — new turn started |
| 06:33:01 | `agentActivity busyCount=0` — finished |
| **06:34:30** | **Host process terminated** — last heartbeat logged, no error, no shutdown message |
| 06:34:30 | srv logs: 4 WebSocket clients disconnected simultaneously (frontend connections dropped) |
| 06:34:30 | `avail_page_gb` jumps to 0.7 GB — Windows freed memory by killing processes |
| 06:44:32 | v0.49.2 launched (user opened new version) |

---

## Root cause

**Windows Out-of-Memory process termination.**

The system's commit charge (page file + RAM) was at **≥99.9%** for ~8 minutes before the
close. `avail_page_gb` sat at 0.1 GB out of 87.7 GB total across that window. When commit
headroom runs out, Windows terminates processes without warning — no crash dialog, no dump,
no log entry from the killed process. The host's own working set was only ~117–124 MB
(AgentMux is not the culprit), but the two agent CLI processes running overnight accumulated
memory that filled the system's page file.

The simultaneous jump in available page file at `06:34:30` (0.1 → 0.7 GB) confirms Windows
was actively reclaiming memory by terminating processes at that moment.

Supporting evidence:
- No crash dump found in `AppData\Local\CrashDumps` or WER
- No `ERROR` or `WARN` in any log around the close time
- Host log ends mid-heartbeat-cycle (20-second intervals; last entry is a normal heartbeat)
- srv (PID 5984) **survived** — proving this was not a machine reboot or power event
- The new session launched 10 minutes later, confirming the machine was never off

---

## Secondary event

At `00:53–00:55 UTC`, agent `c44d6df1` went `Healthy → Stalled → Dead`. This is a separate
earlier OOM event where the agent's subprocess was killed, ~6 hours before the host closed.
The agent was subsequently restarted (it was active again by 06:17 UTC), suggesting automatic
recovery worked. This earlier event may have been the same page-file pressure accumulating.

---

## What was NOT the cause

- Not an AgentMux bug (no error in any log)
- Not a Windows Update reboot (srv stayed alive)
- Not a crash in the Rust host or sidecar (no dump)
- Not a user close (user was away)
- Not network/auth related

---

## Impact

- Both agent panes were mid-run and lost their active turns. Sessions and blockfiles are
  intact (the srv survived and kept writing).
- No data loss beyond the in-flight agent output at time of kill.

---

## Recommendations

1. **Monitor `avail_page_gb` in the sidecar** — alert in the UI when system commit headroom
   drops below ~2 GB. This gives the user a chance to close idle agents before Windows kills
   the host.
2. **Investigate agent process memory** — if two Claude Code panes are accumulating GBs
   overnight, consider idle-timeout or memory-cap tooling.
3. **Consider a host-level OOM handler** — register a low-memory callback
   (`CreateMemoryResourceNotification` on Windows) so the host can do a graceful save-state
   before Windows terminates it.

---

## Log sources consulted

| File | Key finding |
|------|------------|
| `agentmux-launcher.log` | Agent health transitions at 00:53/00:55; WS disconnects at 06:34:30; v0.49.2 start at 06:44 |
| `agentmuxsrv-v0.49.1.log.2026-06-25` | Normal activity through 23:59:59 |
| `agentmuxsrv-v0.49.1.log.2026-06-26` | Activity through 06:34; srv survived past 06:34 |
| `agentmux-host-v0.49.1.log.2026-06-26` (channel: `local-main-b28b7a`) | Last heartbeat at 06:34:30 with `avail_page_gb=0.1`; no error |
| Windows Event Log | Queried; description resolution error prevented reading (permissions issue) |
| Crash dumps | None found |
