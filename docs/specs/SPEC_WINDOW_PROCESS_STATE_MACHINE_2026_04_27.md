# SPEC: AgentMux Window & Process State Machine

**Author:** AgentC (with Explore + research subagents)
**Date:** 2026-04-27
**Status:** Draft for review
**Repo:** `agentmuxai/agentmux` @ `main` (current `0.33.435`)
**Related (read first):**
- `specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` —
  exhaustive map of every state-tracking site in the current code,
  with file:line citations. The "what exists today" companion to
  this spec.
- `specs/ANALYSIS_MULTI_PROCESS_BEST_PRACTICES_2026_04_27.md` —
  prior-art research (Chromium, Electron, VS Code, Tauri, Windows
  Job Objects) that the recommendations below draw from.
- `specs/SPEC_BACKEND_LIFECYCLE.md`
- `specs/process-lifecycle-v2.md`
- `specs/instance-indicator.md`
- Recent merged PRs #562–#568 (tab tear-off + warm pool fixes)

This spec describes a single coherent state machine that owns the truth about
every AgentMux window and every spawned process, replacing the current
patchwork of HashMaps, atomics, polling timers, and fire-and-forget IPC.
It is not a green-field rewrite — section 9 lays out an incremental migration
that lands in 4 phases without freezing feature work.

---

## 1. Problem Statement

AgentMux currently distributes window/process state across at least
**13 distinct stores** in 4 processes (host, launcher, srv, frontend), with
no enforced invariants between them. The recent tab tear-off + warm pool work
(PRs #562–#568) layered new state (pool queue, unpromoted set, in-flight
semaphore, drag session, instance registry) on top of pre-existing
HashMaps without consolidating ownership. Symptoms observed in the wild on
v0.33.435:

| Symptom | Root cause (codebase mapping) |
|---|---|
| **Closing all windows leaves zombie processes** (launcher + multiple host renderers + srv) | The host *does* wrap the srv in a Job Object with `KILL_ON_JOB_CLOSE` (`agentmux-cef/src/sidecar.rs:180-189`, `create_job_object_for_child` at :424), so srv reaping works **when the host exits cleanly**. It doesn't here, because pool windows stay alive in the `browsers` map and keep CEF's message loop running indefinitely after the last visible browser closes — host never exits, job handle never drops, srv is never reaped. The host's own process and its render children are in *no* job, and the launcher creates no jobs at all. |
| **Re-launch shows taskbar entry but no window opens** | Stale `~/.agentmux/data/cef/ipc-port` file outlives the process; new launcher connects to dead IPC server, sends `open_new_window`, gets nothing, exits "successfully". (`agentmux-launcher/main.rs:187-208,382-383`) |
| **Pool windows leak to taskbar** (Phase 6 incomplete) | Promote path clears `WS_EX_TOOLWINDOW` then `SetWindowPos(SWP_FRAMECHANGED)` then `ShowWindow` — Windows requires hide→show *cycle* to refresh the taskbar after a style change. (`window_pool.rs:440`) |
| **Instance count inflates after tear-off crash** | `WindowInstanceRegistry::register()` happens at promote, but `unregister()` is also called from `on_pool_window_destroyed()` on labels that were never registered → silent no-op + leaked instance number. (`window_pool.rs:245-274`, `client.rs:307`) |
| **Tab visible in window A but missing in window B** | Frontend ↔ srv state sync over WPS WebSocket is best-effort; no version stamping, no resync on reconnect. (`frontend/store/global.ts:66-75`) |
| **InstancePanel flickers / shows wrong count** | Frontend polls `list_windows()` every 50ms × 6 after a count-changed event; no per-label event stream from host. (`frontend/app-init.ts:120-160`) |
| **Burst tear-offs empty the pool** | `window_pool_respawn_in_flight: AtomicBool` is `swap(true)` on spawn-start, released on render-ready, not on spawn-complete; second spawn may see stale `false` mid-flight. (`window_pool.rs:239-241`) |

Common thread: **no single owner of "what windows and processes exist right
now"**, no canonical state-transition graph, no event log, and no Job Object
to enforce cleanup as a backstop. Every fix to date has been a local patch
on top of the same fragmented model.

---

## 2. Goals & Non-Goals

### Goals

1. **One canonical store** for the set of `(WindowId, WindowState)` and
   `(ProcessId, ProcessRecord)` per AgentMux instance.
2. **Explicit finite-state machine** for window lifecycle with all
   transitions named, observable, and exhaustively matched.
3. **Pool / warm-window items live in a separate registry** from real
   windows, mirroring Chromium's `SpareRenderProcessHostManager` pattern;
   they enter the canonical list only at the promotion transition.
4. **Guaranteed process-tree cleanup on Windows** via Job Object with
   `KILL_ON_JOB_CLOSE`, regardless of crash mode.
5. **Single-instance enforcement via named mutex**, not a port file.
6. **CQRS-lite IPC contract**: typed commands in, versioned events out,
   resync on reconnect.
7. **Frontend store is a passive mirror** — every mutation is a command;
   every UI change is an echoed event.
8. **No silent drift** — every invariant is checked at transition boundaries
   and violations are observable, not corrupted-and-continued.

### Non-Goals

- Replacing CEF, Tauri-fying the host, or rewriting the srv DB layer.
- Making the srv survive across host restarts — srv stays per-launcher.
- Cross-platform rework. Windows is the target platform; Linux/macOS
  parity work happens in a follow-up. The Job Object choice is Windows
  specific by intent; Linux uses pidfd, macOS uses kqueue (already done).
- Persisting window state across launches (out of scope; covered by
  workspace persistence in srv DB).

---

## 3. Architecture Overview

### 3.1 Ownership: launcher is the privileged owner

Mature multi-process desktop apps (Chromium, Electron, VS Code, Tauri)
universally place the canonical window/tab list in the privileged native
process — it's the only one that holds OS handles whose existence is
ground truth. AgentMux's twist: the **host** process holds HWNDs but can
crash; the **launcher** is the only process guaranteed to outlive a host
crash. So:

- **Launcher** = source of truth for `WindowId`, `WindowState`,
  `ProcessRecord`, `WarmPool`, `LifecyclePhase`. Holds the Job Object.
  Exposes the reducer over a named pipe.
- **Host** = executor. Owns CEF and HWNDs, reports their state. Does not
  decide; only acts on commands from the launcher and emits facts.
- **Srv** = owns persistent app data (workspaces, tabs, blocks). Knows
  nothing about HWNDs. Already a clean split today; keep it.
- **Frontend** = mirror. Subscribes to events, dispatches commands. Never
  authoritative.

This inverts the current arrangement, where the host is authoritative and
the launcher is a thin wrapper. The inversion is what unlocks (a) guaranteed
cleanup via Job Object and (b) survival across host crashes.

### 3.2 Process diagram

```
┌──────────────────────────────────────────────────────────────────┐
│ LAUNCHER (agentmux.exe) — privileged owner                       │
│   • Job Object (KILL_ON_JOB_CLOSE, no breakaway)                 │
│   • Named mutex Local\AgentMux-{guid} for single-instance        │
│   • Reducer:    update(state, Command) → (state, [Event])        │
│   • Stores:     Map<WindowId, WindowState>                       │
│                 Map<ProcessId, ProcessRecord>                    │
│                 WarmPool { ready: VecDeque<HostHandle>,          │
│                            spawning: HashSet<HostHandle> }       │
│                 LifecyclePhase (Starting|Running|Quitting|Dead)  │
│                 EventLog (ring buffer, versioned)                │
│   • IPC server on \\.\pipe\agentmux-{pid}\command                │
│   • IPC server on \\.\pipe\agentmux-{pid}\events  (per-renderer) │
└────┬─────────────────────────────────────────────────────────────┘
     │ assigned to job
     │
     ├──► HOST (agentmux-cef.exe)                                   
     │     Connects to launcher pipe.                               
     │     Receives Commands; emits Facts.                          
     │     Owns CEF + HWNDs.                                        
     │     Stateless WRT canonical registry.                        
     │     Reports HWND ↔ WindowId binding via fact.                
     │                                                              
     │     CEF render-process children inherit the job.             
     │                                                              
     ├──► SRV (agentmux-srv.exe)                                    
     │     Connects to launcher pipe (NEW).                         
     │     Reports started/ready as facts. No heartbeat — death     
     │     is detected via pipe-EOF (kernel guarantees on process   
     │     exit) and child.wait() exit code.                        
     │     Receives Quit command; ack with done.                    
     │                                                              
     │     Already owns workspace/tab DB. No HWND knowledge.        
     │                                                              
     └──► (CEF render-process children — inherit job; tracked       
           by host via on_after_created / on_before_close,          
           reported up as facts.)                                   
```

---

## 4. State Model

### 4.1 Window finite-state machine

```
              spawnFailed
           ┌────────────────┐
           │                ▼
Requested ─►  Spawning  ─► (terminal)
              │
              ▼
          ReadyHidden ──promote──► Visible ──userClose──► Closing ──► Closed
              │                       │                                ▲
              │                       │   crash                        │
              │                       └───────────────────────────────►│
              │ idleEvict                                              │
              ▼                                                        │
           Closed ◄────────────────────────────────────────────────────┘
```

States — owned by the launcher reducer:

| State | Meaning | OS state |
|---|---|---|
| `Requested` | Command accepted, no host action yet | none |
| `Spawning` | Host has been told to create the CEF window | HWND not yet created |
| `ReadyHidden` | HWND exists, renderer signalled ready, taskbar hidden (`WS_EX_TOOLWINDOW`, no `WS_VISIBLE`) | hidden |
| `Visible` | Promoted to user-facing; in canonical window list | visible, on taskbar |
| `Closing` | Either user-close in flight, or graceful teardown started | hidden / animating |
| `Closed` | Fully gone; HWND destroyed; entry removed from registry next tick | none |

Crashes (renderer died, host died) deliver a `Crashed` event into the
reducer, which transitions any state ≠ `Closed` to `Closed` and emits a
`WindowRemoved` fact — there is *no* "crashed but tracked" state. This is
the lesson from Chromium's `WebContentsObserver::WebContentsDestroyed`:
the destroyed signal is the only authoritative one.

### 4.2 Warm pool — separate type

Per Chromium's spare-renderer pattern, pool items are **not in the main
registry**. They live in `WarmPool { ready: VecDeque<HostHandle>, spawning:
HashSet<HostHandle> }`. Pool items have their own state:

```
Spawning ──ready──► Ready ──promoted──► (gone, joins main registry as Visible)
    │                  │
    │                  └── evicted ──► (destroyed, never tracked elsewhere)
    └── failed ──► (semaphore released, refill triggered)
```

Promotion is a **single atomic command** `PromoteWarm { warm_id, become:
WindowId, position }` that:
1. Pops from `ready`.
2. Tells host to clear `WS_EX_TOOLWINDOW`, hide → SetWindowPos → show.
3. Inserts a new entry in the main registry directly at `Visible`,
   skipping `Requested → Spawning → ReadyHidden`.
4. Emits `WindowAdded { id, source: WarmPromote }` event.
5. Issues `RefillPool` command (counted via the pool's own semaphore).

**Invariant:** pool items never appear in `windowAllClosed` checks, never
emit a `WindowAdded` event before promotion, and never share a state-machine
state with main-registry windows. This single invariant kills the Phase 6
"pool windows leak to taskbar" and "pool inflates instance count" bug
classes by construction.

### 4.3 Process model

**What exists today:** the host creates a Job Object with
`KILL_ON_JOB_CLOSE` and assigns the srv to it
(`agentmux-cef/src/sidecar.rs:180-189`, `create_job_object_for_child` at
line 424). When the host exits cleanly, the OS closes the job handle and
reaps the srv. This is real protection — keep it. What it doesn't cover:
the host's own process, the host's CEF render children, and the case
where the host hangs (current zombie scenario: pool windows keep CEF
alive, host never exits, job handle never drops, srv lives).

**What this spec adds:** move the Job Object up one level, into the
**launcher**, and assign *both* host and srv to it (and via inheritance,
all CEF render children). The host's existing srv-job becomes redundant —
either delete it or keep it as defense-in-depth (a reaper at each level
costs nothing).

`ProcessRecord` per child:

```rust
struct ProcessRecord {
    pid: u32,
    kind: ProcessKind, // Host | Srv | Renderer { parent_window: WindowId }
    spawned_at: Instant,
    state: ProcessState, // Spawning | Running | Quitting | Exited { code, at }
    job_assigned: bool,
    handle: HANDLE,
}
```

The launcher creates the Job Object once at startup with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, no `BREAKAWAY_OK`. Every child
(host, srv, and any helper) is launched with `CREATE_SUSPENDED`,
`AssignProcessToJobObject` is called, then `ResumeThread`. CEF render-process
children inherit the job automatically. This is the standard Windows
pattern documented by Microsoft and Raymond Chen; it's what Chrome itself
uses for sandbox-less child cleanup.

The same `CreateJobObjectW` / `SetInformationJobObject` /
`AssignProcessToJobObject` code already exists in
`agentmux-cef/src/sidecar.rs:424`; this work moves the call site to the
launcher and broadens its membership. The host's existing wrap of the srv
can stay (defense-in-depth) or be deleted once the launcher's job is
proven in production.

**Result:** the moment the launcher exits — for any reason, including
hard kill — the OS cleans up the entire tree.

**On heartbeats:** the spec does NOT introduce a host→launcher heartbeat.
Heartbeats exist when state across processes can drift; the whole point
of the reducer + versioned events + GetSnapshot resync (§5) is that it
can't drift. Liveness is detected from authoritative kernel signals:
- Host process death → `child.wait()` returns and/or the launcher's pipe
  to the host returns EOF. Both are kernel-guaranteed at process exit.
- Host message-loop hang while the host process is alive → not a
  liveness issue but a state issue. The reducer's window-all-closed
  predicate (§6.3) excludes warm-pool windows so the host's
  `quit_message_loop` fires correctly when the last visible window
  closes. PR #568 already implemented this in
  `agentmux-cef/src/client.rs::on_before_close`.
- Forced shutdown still bounded → the explicit `Quit { reason }` IPC
  (§5.2) carries a 3s ack timeout; on timeout the launcher
  `CloseHandle(job)` and the OS reaps the tree. This is event-driven,
  not periodic.

The "zombie processes" class of bug goes away because (a) the host
exits normally when it should, (b) when it doesn't, the explicit
shutdown command's timeout backstops it, and (c) when the launcher
itself dies, the kernel reaps everything via `KILL_ON_JOB_CLOSE`.

### 4.4 Single-instance enforcement

Replace the port-file scheme with a **named mutex** plus a named pipe:

- Acquire `Local\AgentMux-{configured-id}` via `CreateMutexW`. If
  `GetLastError() == ERROR_ALREADY_EXISTS`, a previous launcher exists
  and owns the global pipe `\\.\pipe\agentmux-singleton`.
- Send the new launch's command line over the pipe; the existing launcher
  reduces it as a `BringToFront` or `OpenWindow` command.
- Mutex is released by the kernel on process death — no stale-file recovery
  needed.

This is what Electron's `app.requestSingleInstanceLock` does on Windows
(via Chromium's `ProcessSingleton`). It is the standard idiom.

---

## 5. Reducer & IPC Contract

### 5.1 Pure reducer in the launcher

```rust
pub fn update(state: &State, cmd: Command) -> (State, Vec<Event>) {
    // exhaustive match on (cmd, state.lifecycle, relevant entries)
    // total function — no panics, no I/O
}
```

All side effects (spawning processes, posting Win32 messages, talking to
host/srv) are driven by the **events** the reducer produces, executed by
an `Effects` runner that the test suite can swap for a mock. This is the
standard Elm/Redux pattern; it is what makes the system testable. The
choice is deliberate prior art: SAFE-ConfPlanner combines server-side
event-sourcing with an Elm Architecture client, which is the closest
reference for a Rust-launcher + React-frontend split.

### 5.2 Command/Event split (CQRS-lite)

**Commands** (in): typed, validated at the IPC boundary, may be rejected.
- `OpenWindow { workspace_id, position?, source: User|TearOff|Cli }`
- `CloseWindow { id }`
- `PromoteWarm { warm_id, become: WindowId, position }`
- `BringToFront { id }`
- `Quit { reason: UserExit | LastWindowClosed | Forced }`

**Events** (out): emitted *after* the reducer commits. Each carries a
`version: u64` (monotonic per launcher process).
- `WindowAdded { id, source, position, workspace_id, version }`
- `WindowStateChanged { id, from, to, version }`
- `WindowRemoved { id, reason, version }`
- `ProcessSpawned { pid, kind, version }`
- `ProcessExited { pid, code, version }`
- `LifecyclePhaseChanged { from, to, version }`
- `WarmPoolChanged { ready, spawning, version }`

### 5.3 Event delivery — one ordered stream per renderer

Per Chromium's Mojo guidance, ordering is guaranteed *only within a single
pipe*. Each subscriber (host, srv, frontend renderer) gets one ordered
stream from the launcher. Renderers attach via the host (host forwards
the launcher pipe to its renderer over CEF JS bindings).

### 5.4 Resync protocol

On connect or reconnect:
1. Subscriber sends `GetSnapshot()`.
2. Launcher responds with `Snapshot { state, version_at_snapshot }`.
3. Subscriber starts applying `event.version > version_at_snapshot`.
4. If the next event arrives with `event.version > local_version + 1`,
   the stream has a gap → re-snapshot.

This is the standard versioned-snapshot+delta pattern (Chromium Mojo,
event-sourced systems generally). Without it, drift on reconnect is the
single largest source of "ghost windows" in apps of this shape.

### 5.5 Echo-loop guard

When the frontend mirror applies an event that originated from a local
command it just dispatched, set an `applying_remote` flag in the renderer
store so the mutation doesn't re-emit. Per gethopp.app's Tauri-Zustand
sync writeup, omitting this guard is the most common cause of
infinite-loop bugs in this pattern.

---

## 6. Lifecycle Flows (after redesign)

### 6.1 Cold launch

1. Launcher starts. Acquires named mutex (assume success).
2. Creates Job Object.
3. Spawns srv `CREATE_SUSPENDED` → assigns to job → resumes.
   Reducer state: `processes[srv_pid] = ProcessRecord { state: Spawning }`.
4. Spawns host `CREATE_SUSPENDED` → assigns to job → resumes.
5. Host connects to launcher pipe, requests `GetSnapshot`, syncs.
6. Reducer issues `OpenWindow { source: Cli, workspace_id: default }`.
7. Host creates CEF browser; on `on_after_created`, host emits
   `WindowReadyHidden { window_id }` event upstream.
8. Reducer transitions to `ReadyHidden`, then immediately
   to `Visible` (because source != WarmPool), emits
   `WindowAdded`.
9. Frontend renderer attaches event stream, applies, draws.
10. Reducer issues 2× `SpawnPoolWindow` to refill warm pool.

### 6.2 Open second window via tear-off

1. User drags tab.
2. Frontend dispatches `BeginTearOff { tab_id, source_window, cursor }`.
3. Reducer checks `WarmPool.ready`. If non-empty:
   - Issues `PromoteWarm { warm_id, become: new_id, position: cursor }`.
   - Effects runner tells host to clear `WS_EX_TOOLWINDOW`, hide, move,
     show.
   - Reducer commits `WindowAdded { source: WarmPromote }`.
   - Reducer issues `RefillPool`.
4. If empty: reducer falls through to `OpenWindow { source: TearOff }`
   cold path.
5. Frontend in destination window receives `WindowAdded`, runs
   tear-off finalization (`MoveTabToWorkspace` to srv).
6. Source window's tab count drops to 0; frontend dispatches
   `CloseWindow { id: source }`. Reducer transitions through Closing →
   Closed; host destroys HWND; events fan out.

### 6.3 Close last window

1. User closes the last visible window.
2. Reducer transitions it Visible → Closing → Closed.
3. **Window-all-closed predicate** (in the reducer):
   ```
   ∀ w in main_registry: w.state == Closed
   AND lifecycle == Running
   ```
   *Pool windows are NOT in `main_registry`, so they don't block the
   predicate — by construction.*
4. Predicate true → reducer transitions `LifecyclePhase: Running → Quitting`,
   emits `LifecyclePhaseChanged`.
5. Effects runner:
   - Issues `Quit { reason: LastWindowClosed }` to srv over pipe; awaits
     ack with timeout (e.g. 3s).
   - Issues `Shutdown` to host; awaits CEF message-loop drain.
   - Drains pool (sends `EvictAll` to host).
6. After both ack (or timeout), launcher `CloseHandle(job)` → OS guarantees
   the rest of the tree is dead.
7. Launcher releases mutex, exits.

**Result:** zero leaked processes. Even if the host hangs and the timeout
fires, `CloseHandle(job)` with `KILL_ON_JOB_CLOSE` cleans up everything.

### 6.4 Re-launch immediately after exit

1. Old launcher exits → mutex released atomically by kernel.
2. New launcher acquires mutex (succeeds).
3. No port file, no race, no "taskbar but no window".

---

## 7. Invariants (checked at boundaries)

The reducer enforces these on every transition; violations panic the
launcher (which the OS recovers via Job-Object cleanup — preferred to
silent corruption):

1. `∀ w ∈ main_registry: w.id ∉ warm_pool.ready ∪ warm_pool.spawning`.
2. `Visible` ⇒ HWND exists and `WS_VISIBLE` set.
3. `ReadyHidden` ⇒ `WS_EX_TOOLWINDOW` set, `WS_VISIBLE` not set.
4. `WindowRemoved` event ⇒ `processes[*].kind == Renderer { parent: id }` are
   all transitioned to `Exited`.
5. `LifecyclePhase == Quitting` ⇒ no new `OpenWindow` accepted.
6. Event stream is dense and monotonic per launcher run: no gaps, no
   duplicates, no decreasing versions. Frontend uses this to detect drops.

---

## 8. Anti-patterns we are explicitly killing

| Anti-pattern | Where in current code | Replacement |
|---|---|---|
| Fire-and-forget `child.kill()` (redundant given the existing srv-job, but ineffective once the host hangs) | `agentmux-cef/main.rs:371-372` | Launcher-owned job handles it; explicit `Quit` command with ack and timeout, then `CloseHandle(job)` as backstop |
| `MutexGuard` held across awaits | `AppState.browsers` locked through CEF callbacks doing I/O | Reducer is sync + pure; effects run on a separate runtime; no locks cross awaits |
| Polling for state changes | Frontend `app-init.ts:120-160` | Event stream + snapshot resync |
| Ad-hoc parallel HashMaps | `browsers`, `window_meta`, `window_id_map` in `AppState` | Single `Map<WindowId, WindowState>` with `WindowState` carrying everything |
| Pool windows mixed into main map | `unpromoted_pool_labels: HashSet` set parallel to `browsers` | Pool is a separate type that never enters main registry until promote |
| Stale port file for single-instance | `agentmux-launcher/main.rs:187-208` | Named mutex |
| Two sources of truth for "instance count" | `WindowInstanceRegistry::count()` vs frontend `openWindowLabels.length` | Frontend mirrors registry directly; only one count exists |
| Soft-closed / hidden-but-pretending-closed windows | Pool windows alive in `browsers` but filtered out everywhere | Pool windows in their own registry, never in `browsers` |

---

## 9. Migration Plan (4 phases, no feature freeze)

The redesign is intrusive but can land incrementally. Each phase is shippable.

### Phase A — Foundation (1–2 sprints)

- **Move the existing Job Object up to the launcher.** The Win32 calls
  already live in `agentmux-cef/src/sidecar.rs:424`; lift them into
  `agentmux-launcher/src/main.rs`, assign both the host and the srv at
  spawn time. Spawn each child with `CREATE_SUSPENDED`,
  `AssignProcessToJobObject`, then `ResumeThread` so children created
  during host startup can't escape the job (the standard Microsoft /
  Raymond Chen pattern; codex/gemini correctly flagged the race in the
  initial PR #570 implementation). Keep the host's srv-job in place as
  defense-in-depth for the first release, then remove.
- Replace port-file single-instance with **per-data-dir named mutex** +
  named pipe. Mutex name `Local\AgentMux-{hash16(canonical_lower(data
  _dir))}`; multi-instance support per `CLAUDE.md` is preserved by
  scoping the mutex to data-dir, not globally. Mutex is kernel-released
  on launcher death so stale-port-file recovery vanishes.
- Defer `LifecyclePhase` enum and synchronous `Quit { reason }` IPC to
  Phase B alongside the rest of the reducer surface. They are part of
  the central architectural inversion (truth → launcher) and a
  "minimal" Phase A version would be re-architected when Phase B
  arrives. The existing host-side `quit_message_loop` (PR #568) keeps
  the happy path working in the meantime.

**Explicitly NOT in Phase A**: any periodic heartbeat or polling
timer between launcher and host. The state-machine architecture is
the *replacement* for those mechanisms — see §4.3 "On heartbeats"
above.

**User-visible win:** zombies become recoverable (Task Manager kill of
launcher → kernel reaps everything via `KILL_ON_JOB_CLOSE`); relaunch
race ("taskbar but no window" from stale port file) goes away. Two
focused PRs (Job Object + race fix; named mutex + handoff).

### Phase B — Window state machine (2–3 sprints)

- Introduce `WindowId` (newtype over UUID; replace string-label keys
  internally).
- Stand up the pure reducer + effects runner in the launcher.
- Migrate the existing `browsers` / `window_meta` / `window_id_map` /
  `window_instance_registry` HashMaps into one `Map<WindowId,
  WindowState>` owned by the reducer.
- Host becomes a thin executor: it sends facts up, receives commands down.
- Frontend keeps its current store but now subscribes to the event
  stream; polling loop in `app-init.ts` is deleted.

### Phase C — Warm pool consolidation (1–2 sprints)

- Move `window_pool` / `unpromoted_pool_labels` /
  `window_pool_respawn_in_flight` into a typed `WarmPool` struct
  inside the reducer.
- Promotion becomes a single command. Phase 6 taskbar bug goes away
  because hide → SetWindowPos → show is now a single effect with a
  guaranteed order.
- Refill driven by reducer events, not by ad-hoc semaphore.

### Phase D — IPC contract hardening (1 sprint)

- Versioned events; `GetSnapshot` resync protocol.
- Echo-loop guard in frontend store.
- Mojo-style separate ordered pipes per subscriber.
- Invariant assertions enabled in debug builds; downgraded to metrics
  in release.

---

## 10. Testing & Observability

- **Reducer is pure** ⇒ exhaustive unit tests for every (state, command)
  pair, every invariant. Property-based tests (`proptest`) for sequences
  of commands check invariants hold across arbitrary orderings.
- **Effects runner** has a mock backend for integration tests that drive
  full lifecycle scenarios without spawning real CEF.
- **Event log** kept as a ring buffer of last 1000 events with a debug
  command to dump on demand. Every bug report can include this.
- **Process inventory** exposed by the launcher: `agentmux.exe --diag`
  prints the canonical state + the actual `Get-Process` view, with
  diff. CI runs this after a synthetic close-all and asserts equality.
- **Metrics:** transition counters per (from, to), promotion latency,
  pool size over time, time-to-window-visible from cold launch.

---

## 11. Open Questions

1. **Should the launcher persist event log across runs?** Useful for
   crash forensics. Probably yes, with rotation; a follow-up.
2. **Do we want the host to be restartable while the launcher stays up?**
   This architecture makes it possible (launcher would respawn the host
   and replay state), but it's a non-trivial UX decision (does the user's
   work survive?). Defer.
3. **Should srv subscribe to the window event stream?** Currently no — srv
   is window-agnostic. Could be useful for "close window N's tabs from
   the API" features. Defer.
4. **How does this interact with existing spec
   `process-lifecycle-v2.md`?** Need to read and either supersede or
   harmonize. Open before merging this spec.
5. **Browser-pane lifecycle** (embedded CEF child views inside main
   windows) — current code treats them as labels in `browsers` map.
   Should they be in the canonical registry or owned by their parent
   window's state? Lean toward the latter.
6. **CEF render-process crashes** — Chromium reaps renderers and
   notifies after handle invalidation; we need to mirror that with
   handle-duplication before notifying observers, per Chromium dev
   list discussion.

---

## 12. References

Codebase mapping (2026-04-27, AgentC Explore agent):
- `agentmux-cef/src/state.rs:41-281`, `client.rs:97-350`,
  `commands/window_pool.rs:1-525`
- `agentmux-launcher/src/main.rs:1-158`
- `agentmux-srv/src/main.rs:28-236`
- `frontend/app-init.ts:120-160`, `app/init/pool.ts:17-75`,
  `app/statusbar/InstancePanel.tsx:1-180`

Best-practice prior art (2026-04-27, research agent):
- Chromium tab helpers / `BrowserList` / `TabStripModel`
- Chromium `SpareRenderProcessHostManager` (codereview.chromium.org/2929113002)
- Chromium Mojo IDL versioning + style guide
- Electron `BrowserWindow`, `app.requestSingleInstanceLock`
- VS Code `IWindowsMainService` / `LifecycleMainService`
- Tauri state management; `gethopp.app` Zustand-Tauri sync writeup
- Microsoft Job Objects docs; Raymond Chen "Destroying all child
  processes when the parent exits"; Meziantou Job Object writeup
- Elm CQRS / Intents-and-Facts (Elm discourse); SAFE-ConfPlanner
  reference implementation
- Rust crates: `cqrs-es`, `esrs`
