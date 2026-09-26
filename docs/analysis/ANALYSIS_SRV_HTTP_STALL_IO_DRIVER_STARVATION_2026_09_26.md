# Analysis: srv stops answering HTTP under host load — Tokio I/O-driver starvation

**Date:** 2026-09-26
**Status:** analysis — root-cause mechanism confirmed from live stack dumps; the
§6 fixes shipped in #3826; §8.3 in #3834; §8.1–§8.2 in #3837; §9 (keeping
AgentMux responsive under agent load) lists further proposals.
**Author:** AgentY, at the request of the repo owner
**Related:** `docs/specs/SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03.md` (the
liveness probe that recycled srv), `docs/specs/SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md`
(the recycle path), `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` and
`docs/specs/SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07.md` (the banner system §8.2 reuses).

---

## 1. Symptom

A running instance (v0.57.2) "restarted on its own" three times in one day: every
pane reloaded and every agent session was marked interrupted and resumed. The
launcher log is explicit each time:

```
[srv-liveness] missed health probe (1 consecutive)
[srv-liveness] missed health probe (2 consecutive)
[srv-liveness] missed health probe (3 consecutive)
[srv-liveness] srv wedged (alive, unresponsive to 3 consecutive health probes) — forcing recycle
srv exited UNEXPECTEDLY (code 1) — respawning srv + recycling host (restart 1/3)
```

The launcher probes srv every 10s with `GET /` and a 3s timeout; three misses in a
row kill srv and, by design, recycle the CEF host too (the host's exit shows as
code 0 because the launcher terminates it — not a host crash). Windows event logs
show no crash, WER or resource-exhaustion entry at any of the three times.

Before the first recycle the probe had been missing single probes intermittently
for ~9 hours (53 misses, up to 14/hour) without ever reaching three in a row.

## 2. What srv was doing: alive, logging, deaf

- srv kept writing routine log lines through every miss. The runtime was running
  tasks; only its sockets were not being answered.
- The UI's own RPCs were late at the same times (`rpc response generator not
  found` — replies arriving after the frontend gave up on them).
- The health handler is trivial (`Json({status, version})`, outside auth, two
  cheap middlewares). Nothing about the endpoint itself can take 3s.

## 3. Live capture

A watcher probed srv every 0.5s with a 1.5s timeout and, on a slow probe, took a
thread-stack minidump (`cdb -pv … .dump /m`, ~1.7s pause). A stall episode
reproduced on the restarted process (04:02–04:04Z: probe latencies up to 5.5s),
and three dumps were taken inside it, plus a healthy baseline.

**Baseline:** exactly one runtime worker (`tokio-rt-worker`, thread `15f40`) sits
in `NtRemoveIoCompletionEx ← GetQueuedCompletionStatusEx` — the worker currently
holding Tokio's I/O driver, dispatching all socket readiness.

**Stalls:** in two of three dumps **no thread** waits on the completion port. The
driver-holding worker was elsewhere:

| Dump | Thread `15f40` |
|---|---|
| 04:02:23 | blocked in `WaitOnAddress` (a Rust mutex) inside a task, under a frame that formats a `block:` MPS scope — i.e. publishing to the broker |
| 04:03:14 | parked normally — the stall had just eased (follow-up probe 1.2s) |
| 04:03:41 | running a task, 30 frames deep: a SQLite write transaction appending an agent's stdin record to its transcript |

No PDB ships with the portable build, so frames were identified from the string
constants each function references (panic locations and tracing strings are
embedded in release binaries): the 04:03:41 frames reference `BEGIN IMMEDIATE` +
`rusqlite-0.31`, and `filestore append` / `stdin` / `…\shell\file_ops.rs:42`; the
top frames reference strings inside SQLite's C code (`rows updated`, `-- %s`).

### 3.1 Mechanism

Tokio's multi-thread scheduler has one I/O driver, polled by whichever idle worker
parks on it. Idle workers park on a condvar, not the driver. When the worker
holding the driver is woken to run a task and that task **blocks synchronously**,
nobody polls the driver until that worker parks again: no socket is serviced —
new connections, the health endpoint, the frontend's WebSocket — while timers and
already-runnable tasks continue. That is exactly "alive in the logs, deaf on the
wire".

A synchronous call that is fast on an idle machine (a SQLite transaction, a lock,
a few hundred syscalls) becomes seconds when the host is saturated, and every
second of it is a second of total network silence if it lands on the driver
worker.

## 4. Blocking work found on async workers

1. **Message delivery persistence.** `run_agent_turn` (`async fn`) →
   `send_message_from` → `persist_message_to_blockfile` → `persist_user_line`:
   two SQLite write transactions (the block's store, and the host-global
   transcript store every instance writes to), each with a 5s busy timeout,
   inline on the runtime. Same via App API `agent.send`. This is the 04:03:41
   stack.
2. **MPS broker publish.** `Broker::publish` holds one process-wide mutex while it
   clones the event into persisted history (once per scope) and hands it to
   `EventBusBridge::send_event`, which serialized the payload into a `Value` and
   then deep-copied it a second time in `send_to_conn_lane`. Every publisher in the
   process funnels through that lock. Consistent with the 04:02:23 stack.
3. **Process-tracker poller.** `spawn_poller` runs `poll_and_emit()` every 2s
   inside `tokio::spawn`: per-process `OpenProcess`/`QueryFullProcessImageNameW`/
   memory queries for every agent's job-object members plus a system-wide
   Toolhelp snapshot, under two mutexes. ~20ms on an idle machine (528 system
   processes, 141 under this srv's agents) — cheap, but unbounded under load.
4. **Lease renew/release** (`subprocess/host_spawn.rs`): a blocking `LockFileEx`
   plus an fsync'd write inside `tokio::spawn`. Not active in this incident
   (leasing is a no-op when `instance_id` is empty), same class.

Correctly handled already, for contrast: memory attribution
(`block_in_place`), `blockfile:line_count` (`spawn_blocking`), the persistent
stdout reader (`spawn_blocking`), 0.57.4's admission lease renewal
(`spawn_blocking`).

## 5. What made it happen when it did

Host load. The afternoon's misses (14:00–23:33Z) coincided with other agents
building and packaging new releases on the same machine; tonight's two stall
episodes coincided with this investigation's own heavy work:

| Stall | Concurrent host load |
|---|---|
| 04:02–04:04Z (1 miss) | recursive scans of all agent workspaces |
| 04:29:47–04:30:07Z (3 misses → recycle) | a normal-priority `cargo test` build of srv on all 32 cores |
| — (0 misses) | the same build re-run at **BelowNormal** priority, 8 jobs |

Also present, as aggravators rather than causes: commit charge at 66/74 GB
(6.4 GB free at the first recycle), and a host-global transcript store of
8.7 GB with a 554 MB WAL (checkpoints not keeping up).

### 5.1 Ruled out

- **Shared-store lock contention alone.** Holding the global transcript store's
  write lock for 3.3s from another process: srv answered every probe in 1–12ms.
- **Message delivery as the afternoon trigger.** 1 of 52 afternoon misses had a
  delivered message in the 13s before it — no better than random windows (3%).
- **A logged operation.** No log event is over-represented before misses.
- **The v0.57.4 instance's peer probing.** Real traffic into this srv, but one
  cheap in-memory request per 0.57.4 turn.

## 6. Fixes in the accompanying PR

1. `backend::blocking::off_async_worker` — `block_in_place` on a multi-thread
   worker, a direct call on a blocking thread or current-thread runtime.
   Synchronous and order-preserving (unlike `spawn_blocking`), which matters for
   #2360's "persist in actual delivery order".
2. `persist_message_to_blockfile` runs its two SQLite writes through it.
3. `spawn_poller` runs `poll_and_emit` via `spawn_blocking(…).await` (still one
   pass at a time).
4. Lease renew → `spawn_blocking`; lease release (which must land before
   `run_lock` drops) → `off_async_worker`.
5. `EventBusBridge::send_event` serializes the event once
   (`rpc_eventrecv_value`) instead of twice — identical wire shape, pinned by a
   test — shortening every broker-lock hold.

Tests: the helper's I/O test runs a one-worker runtime, blocks inside the helper,
and requires a socket round-trip to complete in <800ms. **Negative control:** with
the blocking section inline instead, the same test fails with `round-trip took
1.500525s` — the test detects the defect, not just the fix.

## 7. Follow-ups not in this PR

- **Broker lock scope.** Deliver to routes after releasing the broker mutex (needs
  `client` as `Arc` and a decision on cross-thread ordering between persisted
  history and live delivery). Not done here: it changes ordering semantics and
  the dump did not prove *which* mutex was contended.
- **Ship PDBs** (or publish to a symbol store) for portable builds. This whole
  analysis had to identify functions from embedded strings.
- **Global transcript store health:** an 8.7 GB database with a 554 MB WAL means
  checkpoints are starved — likely a long-lived reader in some instance.

## 8. Proposal (spec): make slowness visible and survivable

### 8.1 The recycle is too aggressive for "slow"

Today: 3 consecutive probes × 3s timeout, 10s apart → kill srv and recycle the
host. That cannot tell a wedged srv from a slow one, and a recycle is expensive —
every agent session is interrupted. Tonight's stalls cleared on their own within
seconds; some probes answered in 5s. Proposed:

- Probe timeout 3s → 10s; recycle only after misses covering ≥ 60s.
- Before killing, require evidence of no progress: srv's CPU time and/or a
  heartbeat counter srv exposes must not have advanced across the miss window.
  Slow-but-progressing is reported (§8.2), not killed.

**Implemented** (launcher, Windows — the only supervisor that runs the srv
liveness probe): `srv_liveness::RecyclePolicy`. Recycle after ≥ 6 consecutive
misses spanning ≥ 55s during which srv used < 250ms of CPU (a sliding window, so
a srv that worked and then deadlocked is still caught within about a minute),
or after 18 consecutive misses (~3 min) regardless. The per-probe timeout stays
3s: the probe runs inside the supervisor's `select!`, so a longer timeout would
stall host-exit detection for that long.

### 8.2 Health-latency telemetry and banner

The probe's round-trip time is itself a health signal. Proposed, reusing the
memory-pressure system end to end:

- **Measure.** The launcher already probes every 10s; record each probe's RTT
  (and treat a timeout as the timeout value). Keep a rolling average (e.g. the
  last 6 probes = 1 minute).
- **Log.** Include the rolling average in the existing liveness log at a low
  cadence (e.g. once a minute, and on every level change) so it can be graphed
  from `agentmux-launcher.log` after the fact.
- **Level.** Feed the rolling average into a debounced tracker with hysteresis —
  the same shape as `memory_pressure::PressureTracker` (Normal / Warn /
  Critical). Suggested thresholds: Warn above 1s average, Critical above 3s
  (the current per-probe timeout); exit thresholds lower than entry to avoid
  flapping.
- **Deliver.** On a level transition the launcher sends an IPC event to the host
  (alongside its existing events); the host emits it to top-level windows with
  `events::emit_event_to_top_level_windows`, exactly as `post_memory_pressure_*`
  does for `memory-pressure`.
- **Show.** A third banner kind next to `<MemoryPressureBanner kind="ram" />` and
  `kind="pagefile"`, same dismiss-per-severity behaviour:
  - Warn: "AgentMux is responding slowly (avg 1.8s). Heavy CPU use by other
    processes, including agents' builds, can cause this."
  - Critical: "AgentMux is barely responding (avg 4.2s) — it may restart itself
    to recover."

**Implemented.** The launcher keeps a 6-probe rolling average (a miss counts as
the 3s timeout) with hysteresis — Warn at ≥ 1s (exit < 0.6s), Critical at ≥ 2.5s
(exit < 1.8s) — logs `[srv-latency]` once a minute and on every level change,
and sends `Command::NotifySrvLatency` to the host, which emits the existing
`memory-pressure` event with `kind: "backend"`. The frontend mounts
`<MemoryPressureBanner kind="backend" />`; same dismiss-per-severity behaviour.

### 8.3 Keep AgentMux responsive under heavy CPU

AgentMux runs at the same priority as the builds its own agents start, so under
saturation it competes equally with `rustc`. Every agent's process tree is
already in a Windows Job Object (`process_tracker`). Setting those jobs'
`JOB_OBJECT_LIMIT_PRIORITY_CLASS` to `BELOW_NORMAL_PRIORITY_CLASS` makes agent
builds and tests yield to AgentMux (and to the user's own foreground apps) under
contention, while still using all idle CPU — builds only slow down when the
machine is actually contended. §5's before/after (normal-priority build → 3
misses and a recycle; the same build at BelowNormal → 0 misses) is the evidence,
one run each. Make it a setting, on by default.

**Implemented.** `agent:belownormalpriority` (settings.json, default `true`, read
live per spawn). Applied to the job of every **agent** process tree — the ACP,
persistent and subprocess controllers, and agent panes' shells — never to a
terminal pane a human types in. The job's limits are read-modify-written so
`KILL_ON_JOB_CLOSE` is preserved. Windows only; a no-op on Linux and macOS.
Not sufficient on its own for a backgrounded window — see §9.3 item 3.

## 9. Keeping AgentMux responsive while agents saturate the machine

The owner's question: *will this ensure that processes started by agents never
freeze AgentMux, and can the CPU charts that break into sections be fixed once
and for all?* This section answers it from the evidence above and from platform
documentation (sources at the end).

### 9.1 "Freezing" is four different failures

| # | Failure | Visible as | Cause |
|---|---|---|---|
| F1 | srv's async runtime starved | server deaf, **chart gaps**, watchdog restarts | blocking work on a runtime worker (§3–§4) |
| F2 | AgentMux threads lose the CPU | everything slow | agents' builds run at the same priority as AgentMux |
| F3 | renderer backgrounded by Chromium | a window stops painting | Chromium drops a background renderer to `IDLE_PRIORITY_CLASS` + EcoQoS |
| F4 | memory pressure | stutters regardless of CPU priority | AgentMux's pages trimmed while builds fill RAM and commit |

Each needs its own fix. None of them alone covers the others.

### 9.2 Why the CPU charts break into sections

The chart draws a break on purpose. `sysinfo-model.ts` inserts `NaN` points
between two consecutive samples more than `max(3000ms, 2.5 × interval)` apart
(`getGapThresholdMs`). Each sample carries **srv's** capture timestamp
(`convertMuxEventToDataItem` reads `event.data.ts`), and no sysinfo events were
dropped on the way (zero `ws egress lane full` warnings in the day's log). So a
break means **srv took no sample for 3 seconds**.

The sampler (`run_sysinfo_loop`) is a task on the same Tokio runtime as
everything else. Its interval is a Tokio timer, and Tokio's timers are driven by
the same driver whose starvation silenced the sockets in §3.1. When F1 happens,
the sampler's ticks do not fire (and `MissedTickBehavior::Skip` then skips them
rather than catching up). **The broken chart and the watchdog restart are the
same event seen twice.** A rendering stall (F3) does not create a break: samples
keep their capture timestamps and are drawn late, not missing.

### 9.3 Practices, and what AgentMux does or should do

**1. Never block the async runtime.** Tokio guidance: async code "should never
spend a long time without reaching an `.await`" — roughly 10–100µs between
awaits — with `spawn_blocking` for blocking I/O, a dedicated thread for
long-running loops, and rayon for CPU-bound work. *Status:* the offenders found
in §4 are fixed in #3826. *Add:* a stall detector, so the next offender is named
instead of rediscovered with a debugger — a watchdog thread that notices the
runtime's own heartbeat task falling behind and logs it.

**2. Separate priorities; do not raise AgentMux to HIGH.** Windows schedules by
strict priority: "if a higher-priority thread becomes available to run, the
system ceases to execute the lower-priority thread". Microsoft warns against
`HIGH_PRIORITY_CLASS` for anything that runs for long ("other threads in the
system will not get processor time"). The right direction is to lower the
work, not raise the UI. *Status:* agents' job objects at `BELOW_NORMAL` (§8.3).
Lower-priority threads are still not starved outright: the scheduler boosts
dynamic priority on I/O completion, input and the foreground window, and
background-mode threads "will never be starved".

**3. Stop Chromium backgrounding AgentMux's own renderers (F3).** In Chromium's
`base/process/process_win.cc`, a best-effort (background) process gets
`IDLE_PRIORITY_CLASS`, and — via `kUseEcoQoSForBackgroundProcess`, enabled by
default — EcoQoS (reduced frequency, efficiency cores). An idle-priority renderer
(base priority 4) loses even to below-normal agent builds (base priority 6), so
**§8.3 alone does not protect a backgrounded window**. AgentMux already disables
`CalculateNativeWinOcclusion` but does not pass `--disable-renderer-backgrounding`
or `--disable-background-timer-throttling` (timers in background pages are
throttled). *Proposed:* pass both, and
`--disable-backgrounding-occluded-windows`. Cost: AgentMux keeps normal priority
and timer cadence when it is not in view — right for a workbench whose value is
watching agents work, and cheap when idle. Make it a setting, on by default.

**4. Memory priority for agents (F4).** CPU priority is not enough; Microsoft's
own remarks: "even an idle CPU priority process can easily interfere with system
responsiveness when it uses the disk and memory". `SetProcessInformation(…,
ProcessMemoryPriority, …)` lowers the priority of a process's pages, so the
memory manager "trims lower priority pages before higher priority pages". It
takes a `PROCESS_SET_INFORMATION` handle, so srv can apply it to agent processes
it tracks. *Proposed:* `MEMORY_PRIORITY_BELOW_NORMAL` for agent processes, applied
by the process-tracker poller as new members appear (the job-object priority
limit covers CPU only). This targets the 66/74 GB commit and the page file
eviction seen during §5's incidents.

**5. I/O.** `PROCESS_MODE_BACKGROUND_BEGIN` lowers CPU, I/O and memory priority
together, but "can be specified only if hProcess is a handle to the current
process", so srv cannot put an agent's build into background mode. Per-process
I/O priority for another process has no documented API. *Not proposed now*;
revisit only if disk contention shows up as its own symptom.

**6. CPU rate control: weight, not a hard cap.** Job objects support hard caps
(`CpuRate`, in 1/100 %), min/max rates, and weight-based sharing (1–9, default 5).
A hard cap "no threads associated with the job will run until the next interval"
— wasting idle cores and slowing every build even on an idle machine. Priority
(§8.3) already makes agents yield under contention while using idle CPU fully.
*Not proposed by default*; a weight option could later balance several agents
against each other.

**7. EcoQoS for agents: no.** Microsoft: EcoQoS is for work "not contributing to
the foreground user experience" and "should not be used for performance
critical" work — it lowers CPU frequency and prefers efficiency cores. Agents'
builds are what the user is waiting for; slowing them to save power is the wrong
trade by default.

**8. Take samples on a dedicated, elevated thread.** Move sysinfo sampling off
the Tokio runtime onto its own OS thread at `THREAD_PRIORITY_ABOVE_NORMAL` —
brief work every interval, which is what Microsoft's guidance allows higher
priority for — timestamping at capture into a small ring buffer that the
runtime drains and publishes. The chart then breaks only when AgentMux itself
could not run for 3 seconds, which makes a break a real alarm instead of noise.
The health-latency banner (§8.2) says the same thing in words.

**9. Other platforms (not verified here).** Linux: the process tracker already
places agents in a cgroup; cgroup v2 `cpu.weight` (and `io.weight`) is the
equivalent of the job priority limit, with `nice`/`ionice` as the per-process
fallback. macOS: the background QoS / `PRIO_DARWIN_BG` policy throttles CPU, I/O
and network together for a process.

### 9.4 Once and for all?

For **load created by agents' processes**, yes — with §8.3 (done), #3826 (done)
and items 3, 4 and 8 above, an agent build cannot take AgentMux's CPU, starve
its server, background its renderer, or evict its pages before its own. What
this cannot guarantee against: another application running at `HIGH` or
`REALTIME` priority, driver or interrupt storms, and genuine memory exhaustion
(when the page file is full, every process stalls). With the layers in place,
those are the only remaining ways to get a gap in the chart — and the gap, the
health banner and the watchdog log will then point at the machine, not at
AgentMux.

### 9.5 Plan

| Item | Where | Status |
|---|---|---|
| No blocking on async workers | srv | done (#3826) |
| Agents' job objects `BELOW_NORMAL` | srv, process tracker | done (#3834) |
| Renderer backgrounding flags | CEF host | proposed |
| Sampler on a dedicated elevated thread | srv sysinfo | proposed |
| Agent memory priority `BELOW_NORMAL` | srv, process tracker | proposed |
| Runtime stall detector | srv | proposed |
| Slow-vs-dead recycle, latency banner | launcher, host, frontend | done (#3837) |

### Sources

- Microsoft, *Scheduling Priorities*: https://learn.microsoft.com/en-us/windows/win32/procthread/scheduling-priorities
- Microsoft, *Priority Boosts*: https://learn.microsoft.com/en-us/windows/win32/procthread/priority-boosts
- Microsoft, *SetPriorityClass* (background mode remarks): https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass
- Microsoft, *SetProcessInformation* (memory priority, EcoQoS): https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation
- Microsoft, *JOBOBJECT_CPU_RATE_CONTROL_INFORMATION*: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_cpu_rate_control_information
- Chromium, `base/process/process_win.cc` (background priority, EcoQoS feature): https://chromium.googlesource.com/chromium/src/+/refs/heads/main/base/process/process_win.cc
- Chrome, *Chrome flags for tools* (renderer backgrounding, timer throttling): https://github.com/GoogleChrome/chrome-launcher/blob/main/docs/chrome-flags-for-tools.md
- Alice Ryhl, *Async: What is blocking?*: https://ryhl.io/blog/async-what-is-blocking/
