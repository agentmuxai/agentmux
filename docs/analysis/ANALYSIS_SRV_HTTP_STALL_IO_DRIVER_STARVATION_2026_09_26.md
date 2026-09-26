# Analysis: srv stops answering HTTP under host load — Tokio I/O-driver starvation

**Date:** 2026-09-26
**Status:** analysis — root-cause mechanism confirmed from live stack dumps; three
fixes (§6) in the accompanying PR; §8 is a proposal (spec), not implemented.
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
