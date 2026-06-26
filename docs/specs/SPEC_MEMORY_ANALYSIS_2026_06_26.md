# Memory Analysis — AgentMux Long-Running Stability
**Date:** 2026-06-26  
**Goal:** Identify what consumes memory during extended sessions and what needs fixing for
6-month continuous uptime.

---

## TL;DR

AgentMux's own processes (host, sidecar, launcher) are memory-stable. The OOM that closed
the app was caused entirely by **Claude Code agent processes**, each of which commits ~10 GB
of virtual address space on Windows. With two concurrent long-running agents the system's
87.7 GB commit budget was exhausted. The fix requires: (1) commit-aware agent scheduling
so the host never spawns a turn it can't back; (2) a low-memory OS-level handler for graceful
degradation; and (3) a RAM upgrade for comfortable 4+ concurrent agent use.

---

## What consumes memory

### Process inventory (observed 2026-06-26, 4 active agent turns)

| Process | Count | WS (each) | VM / commit (each) | Note |
|---------|-------|-----------|---------------------|------|
| `claude` | 4 | 183–377 MB | **~10.5 GB** | One per active turn |
| `agentmux-0.49.2` (CEF renderers) | ~8 | 20–207 MB | ~3.6 TB VA* | Host + renderer fleet |
| `agentmux-0.48.1` (CEF renderers) | ~10 | 17–138 MB | ~2.2 TB VA* | Old version still alive |
| `agentmux-srv-*` | 4 | 8–45 MB | ~4.3 GB VA* | One sidecar per running instance |
| `agentmux-mcp` | 3 | 4–5 MB | tiny | MCP stdio servers |
| `agentmux-bashwrap`, launchers | 6 | 7–11 MB | tiny | Launchers |

*VA numbers for CEF include shared GPU/IPC address space allocations — these do NOT count
against the commit limit. Only "private bytes" (PrivateUsage in PROCESS_MEMORY_COUNTERS)
count.

**Claude Code VM → commit math:**

```
4 agents × 10.5 GB commit = 42 GB
CEF fleet (all versions) private commit ≈ 1–2 GB
OS + other processes ≈ 40+ GB baseline
──────────────────────────────────────────
Total commit charge ≈ 84–85 GB  (observed: 84.7 GB)
Commit budget (RAM + page file): 31.9 + 55.8 = 87.7 GB
Headroom: ~3 GB  ← one more agent turn would exhaust this
```

### Host and sidecar are NOT leaking

Over 30.5 hours of the overnight v0.49.1 session:

| Metric | Session start | Session end | Delta |
|--------|--------------|-------------|-------|
| Host WS (`agentmux-cef`) | 114.4 MB | 124.2 MB | +9.8 MB |
| Host commit | 55.6 MB | 55.6 MB | **0 MB** |
| Host peak WS | 138.7 MB | 138.7 MB | 0 MB |
| Sidecar WS | ~30 MB | ~45 MB | ~15 MB |

The host is perfectly stable. The sidecar is negligible. Neither is a source of leaks.

### Page file drain rate — what actually drained 8.2 GB overnight

```
Jun 25 00:00 UTC   avail_page_gb = 8.3 GB
Jun 25 23:59 UTC   avail_page_gb = 3.8 GB   → lost 4.5 GB in 24h  (~188 MB/hr)
Jun 26 06:00 UTC   avail_page_gb = 2.5 GB   → lost 1.3 GB in 6h   (~217 MB/hr)
Jun 26 06:28 UTC   avail_page_gb = 0.1 GB   → lost 2.4 GB in 28m  (SPIKE — agent activity)
Jun 26 06:34 UTC   Host killed by Windows
```

The baseline drain (~188–217 MB/hr) matches agent turns cycling: each new `claude` process
takes ~10 GB commit but hands back the previous one's commit when the prior turn exits.
However, with two agents active simultaneously, there's a window where both a new turn's
process AND the dying prior process's commit are live at the same time — a brief 20 GB spike.

The spike at 06:28 UTC correlates exactly with agent `c44d6df1` entering a rapid burst of
short turns (new turn every ~60 seconds). Each overlapping start/exit pair transiently held
double the per-agent commit.

---

## What must change for 6-month operation

### P0 — Commit-aware turn scheduler

**Problem:** AgentMux currently spawns agent turns without checking whether the system has
commit headroom to back a new `claude` process (~10 GB).  

**Fix:** Before spawning a new turn, read `GlobalMemoryStatusEx.ullAvailPageFile`. If
`avail < AGENT_COMMIT_RESERVE` (suggested: 12 GB), queue the turn instead of spawning. Show
a "Memory full — waiting…" badge on the affected agent pane. Drain the queue as commit frees
up (poll every 5s or use `CreateMemoryResourceNotification`).

```rust
// In agentmux-srv, before fork/spawn of agent subprocess:
let mem = windows_sys::GlobalMemoryStatusEx();
const RESERVE_GB: u64 = 12;
if mem.ullAvailPageFile < RESERVE_GB * 1024 * 1024 * 1024 {
    return Err(AgentError::CommitPressure { avail_gb: mem.ullAvailPageFile >> 30 });
}
```

This alone prevents the kill: the turn that triggered the fatal burst at 06:28 would have
queued instead.

### P0 — Low-memory OS notification handler

**Problem:** The current `avail_page_gb` warning appears in the UI but disappears before the
user sees it. There's no last-ditch intervention before Windows terminates the host.

**Fix:** In the host or launcher, register a `CreateMemoryResourceNotification` watcher
thread on `LowMemoryResourceNotification`. On signal:

1. Log a structured `OOM_PRESSURE` event with full memory snapshot.
2. Show a **non-dismissable** banner in the frontend: "System memory critical — new agent
   turns paused."
3. Pause all pending turn queues in the scheduler.
4. Optionally checkpoint any active in-flight turn output to disk.

This gives a second layer of protection even if the scheduler misses a spike.

### P1 — Per-agent commit tracking in Swarm view

**Problem:** The Swarm view shows agent health (Healthy/Stalled/Dead) but nothing about
memory. A runaway agent consuming 15 GB of commit is invisible.

**Fix:** Track each spawned agent process via its handle. Call
`GetProcessMemoryInfo(handle, &PROCESS_MEMORY_COUNTERS, ...)` every heartbeat tick and emit
`PrivateUsage` (= private commit bytes) per agent. Surface in the Swarm row as a memory
badge. Alert if any single agent exceeds 12 GB commit.

### P1 — Shut down old version on upgrade

**Problem:** When AgentMux auto-updates from v0.48.1 to v0.49.2, both versions continue
running. Currently 10 CEF renderer processes from v0.48.1 + 8 from v0.49.2 = 18 total
renderers coexisting, doubling the CEF overhead (~1.4 GB WS total vs. ~700 MB expected).
Over months, this can stack multiple old versions.

**Fix:** The launcher (which already owns the Job Object) should send a shutdown signal
to the previous version's job after the new version is confirmed healthy (WS handshake
complete). Respects isolation invariant I3 — only signals within the known job handle.

### P2 — CEF renderer DOM virtualization

**Problem:** As agent output grows to tens of thousands of lines over a long session, the CEF
renderer's DOM holds all of it, growing the renderer's WS proportionally. Measured at 23:06:
block `c44d6df1` had `lines=23069 covered=15838254` (~15.8 MB content). Rendering 23k DOM
nodes in a single scroll container grows the renderer's heap.

**Fix:** Implement virtual scrolling in the agent output pane. Render only a window of N
lines (e.g. 2,000) centered on the current scroll position. Archive off-screen lines to a
background buffer (already persisted in blockfiles). This caps renderer WS regardless of
session length.

### P2 — SQLite WAL checkpoint on idle

**Problem:** After a forceful kill (OOM), WAL files are left uncheckpointed:
`objects.db-wal` = 4.94 MB, `filestore.db-wal` = 4.01 MB. On long runs with high write
volume, WALs can grow into the hundreds of MB and must be replayed in full on next open.

**Fix:** Schedule `PRAGMA wal_checkpoint(TRUNCATE)` on all databases whenever no agent has
been active for N minutes (suggest: 10). Already have the idle signal from `agentActivity`
events.

### P3 — First-run commit budget advisory

For 6-month 4+ concurrent agent use:

| RAM | Page file | Commit budget | Max safe concurrent agents |
|-----|-----------|---------------|---------------------------|
| 32 GB | 56 GB (current) | 87.7 GB | **3** (leaves 17.7 GB margin) |
| 64 GB | 56 GB | 120 GB | **6–7** comfortably |
| 128 GB | system-managed | 150+ GB | unlimited for typical use |

At first launch (or when a new machine is detected), show an advisory if:
`total_commit_budget < 90 GB` — recommend enabling "system managed" page file or upgrading
RAM for heavy multi-agent use.

---

## Summary of what to build

| Priority | Item | Where | Estimated scope |
|----------|------|--------|----------------|
| **P0** | Commit-aware turn scheduler | `agentmux-srv` | ~150 LoC Rust |
| **P0** | `CreateMemoryResourceNotification` handler | host (`agentmux-cef`) | ~80 LoC Rust + 1 frontend event |
| **P1** | Per-agent commit in heartbeat + Swarm display | srv + frontend | ~100 LoC Rust, ~50 LoC SolidJS |
| **P1** | Shut down old version on upgrade | launcher | ~50 LoC Rust |
| **P2** | Virtual scroll in agent output pane | frontend | medium (existing virtual scroll libs) |
| **P2** | WAL checkpoint on idle | srv | ~20 LoC Rust |
| **P3** | First-run commit budget advisory | host/frontend | ~30 LoC |

---

## Log evidence

| Source | Finding |
|--------|---------|
| `agentmux-host-v0.49.1.log.2026-06-25` | Host commit flat at 55.1–55.6 MB for 24 hours |
| `agentmux-host-v0.49.1.log.2026-06-26` | `avail_page_gb` = 0.1 at 06:28–06:32; host ws=117–124 MB |
| `Get-Process claude` (live, 2026-06-26) | 3 processes, each VirtualMemorySize64 ≈ 10.5 GB |
| `Get-CimInstance Win32_PageFileUsage` | AllocatedBaseSize=57149 MB; CurrentUsage=87 MB (paged-out only 87 MB, but commit near-full) |
| `Get-CimInstance Win32_OperatingSystem` | ullAvailVirtual = 3 GB available commit (live) |
