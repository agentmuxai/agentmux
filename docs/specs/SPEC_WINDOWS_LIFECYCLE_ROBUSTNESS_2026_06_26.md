# Windows Lifecycle Robustness — Surviving External Termination
**Status:** P0 partially implemented (see §5); P1 proposed  
**Date:** 2026-06-26  
**Author:** AgentA  
**Adversarial review:** 2026-06-26 (13 findings; see §7)  
**Motivating incident:** `docs/incident/INCIDENT_2026_06_26_APP_CLOSED.md`  
**Complements:** `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`

---

## 1. Problem statement

AgentMux targets 6-month continuous uptime on a Windows desktop. Windows can
externally terminate the host process in ways the current lifecycle doesn't
handle. The OOM case is addressed by `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`
(P0 already shipped in `mem_supervisor.rs`). This spec addresses the remaining
lifecycle gaps and formalises the "what survives a kill" contract.

---

## 2. Kill taxonomy

| Scenario | Notification | Who survives | Current handling |
|---|---|---|---|
| **OOM / Chromium `0xe0000008`** | Chromium self-raises | srv survives (sibling) | **Handled** — `mem_supervisor.rs` |
| **Windows shutdown / logoff** | `WM_QUERYENDSESSION` → `WM_ENDSESSION` to windows with HWNDs | J0 kills all at `WM_ENDSESSION` | **P1** (see §4.A) |
| **Power suspend** | `WM_POWERBROADCAST`→`PBT_APMSUSPEND` before suspend | All frozen; wake resumes them | **P1** (see §4.B) |
| **Task Manager kill** | None | srv survives (sibling in J0, not child of host) | Partial — srv survives; see §6 |
| **Clean shutdown (Ctrl+C, SIGTERM)** | Signal / `SetConsoleCtrlHandler` | Launcher controls teardown | Handled — extended by §4.C |
| **BSOD / power failure** | None | Nothing | Out of scope — durability in `objects.db` |

---

## 3. Existing foundations (unchanged)

- **`persist_subscriber.rs`** — synchronous per-event SQLite write; no batch delay.
  Race window to disk is ~one tokio yield (~100 µs). Workspace is crash-durable.
- **`mem_supervisor.rs`** — OOM classification, commit-gated backoff relaunch,
  graceful give-up dialog. Already shipped.
- **Job Object J0 (`KILL_ON_JOB_CLOSE`)** — atomically reaps the entire instance
  tree when the launcher exits. SQLite is per-instance (`~/.agentmux/channels/<ch>/`)
  so multiple concurrent instances have no WAL contention.
- **SQLite WAL + `busy_timeout=5000`** — `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;`
  set at open in both `store.rs` (objects.db) and `filestore/core.rs`. A `wal_checkpoint`
  that hits a reader waits up to 5s then returns partial-truncate — never blocks forever,
  never errors fatally.

---

## 4. Design

### 4.A — Windows shutdown / `WM_QUERYENDSESSION` *(P1 — deferred)*

**Why deferred.** Implementing this correctly requires two prerequisites that
are not yet in place:

1. **The launcher is `#![windows_subsystem = "windows"]` (GUI subsystem).**
   `SetConsoleCtrlHandler`/`CTRL_SHUTDOWN_EVENT` only fires in console-subsystem
   processes — it will never fire in the launcher. A GUI-subsystem process only
   receives `WM_QUERYENDSESSION` if it has a message loop with an HWND. The
   launcher has no persistent HWND after the splash is dismissed.
   Fix: add a hidden message-only window (`CreateWindowExW` with
   `HWND_MESSAGE` as parent) to the launcher, registered at startup, that
   receives `WM_QUERYENDSESSION` and signals the supervision loop.

2. **No `LifecycleEvent` channel between host and srv.** Checkpointing srv's
   SQLite from the host requires a new IPC message type. Options: (a) new
   message variant on the existing host pipe protocol, (b) srv exposes a local
   HTTP endpoint the host can hit, (c) host injects JS into its own renderer
   which sends a WebSocket message to srv. Option (c) reuses existing
   infrastructure but adds a round-trip. This design choice needs a separate PR.

**When implemented, the correct approach is:**

1. Register `ShutdownBlockReasonCreate(hwnd, "Saving agent sessions…")` **at
   startup** (not inside the handler — the handler is already inside the
   shutdown window). Note: `ShutdownBlockReasonCreate` changes the text shown
   in the Windows shutdown-blocker UI but does **not** extend the deadline;
   the hard ceiling is `HungAppTimeout` (default 5s, enterprise GPO can lower
   it to 1s).
2. On `WM_QUERYENDSESSION`: send `LifecycleEvent::Checkpoint` to srv
   **asynchronously** (post a message, return immediately from the handler —
   never block the UI thread for up to 3s, which would trigger the hung-app
   killer). The handler must return TRUE promptly.
3. A background thread waits for the checkpoint ACK (bounded: 3s). On ACK or
   timeout, call `ShutdownBlockReasonDestroy(hwnd)`.
4. On `WM_ENDSESSION(TRUE)`: the launcher calls
   `saga_coord.cancel_all_in_flight("system shutdown").await` before
   `drop(job)`.

### 4.B — Sleep / hibernate / resume *(P1)*

Register for `WM_POWERBROADCAST` in the host's Win32 message pump.

**On `PBT_APMSUSPEND`:**
- Send `LifecycleEvent::Suspend` to srv (same new channel as §4.A).
- Srv appends `{type:"suspend", ts:…}` to each active blockfile so agent
  history shows where output was cut.
- Pause CEF's tick loop to avoid GPU errors on wake.

**On `PBT_APMRESUMESUSPEND`:**
- Do NOT sleep a hardcoded duration waiting for network (network recovery is
  adapter/driver-dependent; use `NotifyAddrChange` if network readiness
  matters). Instead send `LifecycleEvent::Resume` to srv immediately.
- Srv inspects active agent subprocess handles: if alive, the agent survived
  the freeze; if dead, emit a `turn_interrupted` event so the frontend shows
  "Turn interrupted by sleep/wake".
- Re-check `commit_free_mb()` — hibernate can shift page file state.

### 4.C — Saga cancel on clean shutdown *(P0 — implemented)*

**Problem.** On any clean launcher exit (user quit, SIGTERM, OOM give-up), the
in-flight saga registry is discarded with open `SagaStarted` brackets in the
durable log. LSD-3 compensation on next startup tries to compensate these,
which may produce spurious recovery actions for sagas that actually completed
their work.

**Fix — implemented.** Added `SagaCoordinator::cancel_all_in_flight(reason)` in
`agentmux-launcher/src/saga/mod.rs`. It drains the in-flight registry, emits
`SagaFailed` on the broadcast bus for each (closes the bracket for live
subscribers), and writes `terminate_saga(Failed, reason)` directly to the
durable log (bypasses task scheduling — safe during shutdown teardown).

Called from both the Unix shutdown path (before `terminate_child_gracefully`)
and the Windows shutdown path (before `drop(job)`), ensuring saga brackets are
closed before J0 is dropped.

### 4.D — Periodic WAL checkpoint *(P0 — implemented)*

**Problem.** After a forceful kill the WAL is left uncheckpointed. On long
sessions with high agent write volume (blockfiles written every ~400ms), WAL
files grow into the MB range and must be replayed on next open.

**Fix — implemented.** Spawned a background task in `agentmux-srv/src/main.rs`
that runs `PRAGMA wal_checkpoint(TRUNCATE)` on both `objects.db` (wstore) and
`filestore.db` every 30 minutes while the srv is running. Cancelled cleanly
on the `stdin_token` shutdown signal.

`Store::checkpoint()` and `FileStore::checkpoint()` methods added. The
existing `busy_timeout=5000` handles reader contention — partial truncate on
contention is safe and picked up on the next pass.

### 4.E — WER / minidump for the host *(P1)*

Register an unhandled-exception filter in the host (`agentmux-cef`) to write
a minidump on unexpected termination:

```rust
// In WinMain, after logging is initialised:
SetUnhandledExceptionFilter(seh_crash_handler);
// seh_crash_handler writes a minidump to ~/.agentmux/crash-dumps/host-<ts>.dmp
// then re-raises.
```

**Do NOT use `WerAddExcludedApplication`** — that suppresses WER reports
(opposite of intent). Use `SetUnhandledExceptionFilter` or hook into CEF's
bundled Crashpad completion callback. Note: CEF installs its own
`SetUnhandledExceptionFilter`; wiring on top requires care to chain the
existing filter rather than replace it.

### 4.F — Sticky memory pressure banner *(P1)*

The memory warning "came and went" because it's a toast. When
`avail_page_gb < 1 GB` (WARN) or `< 512 MB` (CRITICAL):

- Emit a `MemoryPressure { level, avail_gb }` WPS wave event from the host
  (from the existing `memory_heartbeat.rs` data — no new measurement).
- Frontend `ErrorBanner.tsx` responds:
  - **WARN**: dismissable amber banner, resurfaces every 5 min while still low.
  - **CRITICAL**: non-dismissable red banner pinned to all windows; only clears
    when commit returns above WARN threshold.

The `commit_free_mb()` value is already read every 20s in the host. The only
new work is emitting it as a WPS event instead of only to the log.

---

## 5. Implementation status

| Item | Status | Files |
|---|---|---|
| `SagaCoordinator::cancel_all_in_flight` | **Shipped** | `agentmux-launcher/src/saga/mod.rs` |
| `cancel_all_in_flight` wired on Unix shutdown | **Shipped** | `agentmux-launcher/src/main.rs` ~1258 |
| `cancel_all_in_flight` wired on Windows shutdown | **Shipped** | `agentmux-launcher/src/main.rs` ~1869 |
| `Store::checkpoint()` | **Shipped** | `agentmux-srv/src/backend/storage/store.rs` |
| `FileStore::checkpoint()` | **Shipped** | `agentmux-srv/src/backend/storage/filestore/core.rs` |
| Periodic WAL checkpoint task in srv | **Shipped** | `agentmux-srv/src/main.rs` |
| WM_QUERYENDSESSION handler | Deferred P1 | Requires hidden HWND + LifecycleEvent channel |
| Sleep/resume handler | Deferred P1 | Requires LifecycleEvent channel |
| Host WER / minidump | Deferred P1 | Requires Crashpad chain |
| Sticky memory pressure banner | Deferred P1 | Requires WPS event from heartbeat |

---

## 6. The "what survives" contract

| State | Survives any kill? | Mechanism |
|---|---|---|
| Workspace layout (panes, tabs, positions) | Yes | `objects.db` written per-event |
| Agent histories (blockfile content) | Yes | FileStore (global + per-channel) |
| Active agent turn — output received so far | Yes | Blockfile flushed per write |
| Active agent turn — in-flight LLM chunk | **No** — partial chunk lost | LLM API is stateless; user re-sends |
| Agent turn queued but not started | No — lost on kill | Agent re-queues turn manually |
| In-flight launcher sagas | **Closed cleanly on graceful exit** (§4.C); lost on kill (OOM, Task Manager) | `cancel_all_in_flight` on graceful; LSD-3 compensates on kill |
| Shell session (terminal contents) | No — process state | Expected; no change |
| User sessions / API keys | Yes | Global auth store (cross-channel) |

---

## 7. Adversarial review disposition (2026-06-26)

13 findings raised; disposition below.

| # | Finding | Disposition |
|---|---|---|
| F1 | `ShutdownBlockReasonCreate` doesn't extend grace period | **Fixed in spec** — §4.A corrected: it changes UI text only; 5s hard ceiling is `HungAppTimeout` |
| F2 | 3s sync wait on UI thread triggers hung-app kill | **Fixed in spec** — §4.A now specifies async handler; deferred to P1 |
| F3 | `Relaxed` atomic ordering should be `Release/Acquire` | **Fixed in implementation** — use `Release`/`Acquire` where applicable |
| F4 | `CTRL_SHUTDOWN_EVENT` won't fire (launcher is GUI subsystem) | **Confirmed, fixed in spec** — §4.A deferred; hidden HWND required |
| F5 | `cancel_all_in_flight` undefined | **Implemented** — §4.C + `saga/mod.rs` |
| F6 | `LifecycleEvent` channel undefined | **Accepted, deferred** — §4.A/§4.B marked P1; channel design is a separate PR |
| F7 | Multi-instance SQLite contention | **Non-issue** — SQLite DBs are per-instance (`channels/<ch>/`); no shared file |
| F8 | `wal_checkpoint(TRUNCATE)` may block on `SQLITE_BUSY` | **Already handled** — `busy_timeout=5000` set at open; partial truncate is safe |
| F9 | J0 race: launcher exit kills srv before checkpoint | **Mitigated** — `cancel_all_in_flight` writes directly to the log (no bus delivery dependency); WAL periodic checkpoint (§4.D) reduces the gap. Full solution in P1 §4.A ACK flow |
| F10 | Hardcoded 2s sleep on resume | **Fixed in spec** — §4.B removes sleep, uses `NotifyAddrChange` if needed |
| F11 | `WerAddExcludedApplication` does the opposite | **Fixed in spec** — §4.E now specifies `SetUnhandledExceptionFilter` |
| F12 | Memory thresholds are absolute, wrong field | **Clarified** — thresholds use `ullAvailPageFile` (commit limit), consistent with `mem_supervisor.rs`; absolute thresholds acceptable (512 MB / 1 GB are appropriate floor values for this workload) |
| F13 | §6 "what survives" contract incomplete | **Expanded** — §6 now covers queued turns, in-flight sagas, and LLM chunks |

---

## 8. Out of scope

- BSOD / power failure — covered by existing SQLite durability.
- Enforced cross-instance memory budget — violates isolation invariants I2/I3.
- macOS / Linux equivalents (`NSWorkspaceWillSleepNotification`, SIGTERM on systemd) — deferred.
- Agent turn continuation after kill — LLM API is stateless; no checkpoint-restart.

---

## 9. Cross-references

| Document | Relationship |
|---|---|
| `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` | P0 OOM restart (ships first) |
| `docs/incident/INCIDENT_2026_06_26_APP_CLOSED.md` | Motivating incident |
| `docs/specs/SPEC_MEMORY_ANALYSIS_2026_06_26.md` | Root-cause analysis |
| `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` | Supervision contract |
| `SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` | I1-I6 invariants |
| `agentmux-launcher/src/mem_supervisor.rs` | OOM classifier |
| `agentmux-srv/src/crash_monitor.rs` | Minidump model for §4.E |
