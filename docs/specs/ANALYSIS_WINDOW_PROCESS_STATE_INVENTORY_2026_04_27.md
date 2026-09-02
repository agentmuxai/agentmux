# ANALYSIS: AgentMux Window/Process State Inventory

**Date:** 2026-04-27
**Author:** AgentC (via Explore subagent)
**Purpose:** Background research feeding
`SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`. Maps every place in
the current codebase that tracks window, tab, instance, or child-process
state, with file:line citations.

This is a snapshot of `main` at the time of writing (commit
`f6cfc7e8`, version `0.33.435`). It is descriptive — solutions are in the
spec, not here.

---

## State Inventory

| State Piece | Owner | Type | Key Holder | Mutation Entry Points | Sync Mechanism |
|---|---|---|---|---|---|
| Active CEF browsers | Host (`client.rs`) | `HashMap<String, Browser>` keyed by window label (`"main"`, `"window-{uuid}"`, `"browser-pane-*"`) | `AppState.browsers` (Mutex) | `on_after_created` (insert), `on_before_close` (remove) | IPC `window-instances-changed` broadcast to all windows |
| Window metadata (FullInstance vs Subwindow) | Host (`state.rs`) | `HashMap<String, WindowMeta>` (label→{kind, parent_id}) | `AppState.window_meta` (Mutex) | Pool spawn, `on_after_created`, `on_before_close` (cascade deletes) | Implicit; metadata drives taskbar visibility via Win32 `WS_EX_TOOLWINDOW` |
| Window instance registry (sequential numbers) | Host (`state.rs`) | `WindowInstanceRegistry` (HashMap label→u32, next_num counter) | `AppState.window_instance_registry` (Mutex) | `register()` on pool promote (`window_pool.rs:484`), `unregister()` on close (`client.rs:307`) | Broadcast after promote (`window_pool.rs:493-497`) |
| Pre-warmed pool windows | Host (`window_pool.rs`) | FIFO `VecDeque<String>` of ready labels + HashSet of unpromoted labels | `AppState.window_pool`, `AppState.unpromoted_pool_labels` (both Mutex) | `spawn_pool_window()` (adds to unpromoted set), `mark_pool_window_renderer_ready()` (moves to pool queue), `promote_pool_window()` (pops), `on_pool_window_destroyed()` (cleanup) | Implicit; frontend waits for `pool:promote` event before initializing |
| Pool refill in-flight semaphore | Host | `AtomicBool` (AcqRel ordering) | `AppState.window_pool_respawn_in_flight` | `swap(true)` on spawn start, `store(false)` on ready/destroy | Fire-and-forget: prevents duplicate spawns during backlog |
| Pending window labels (pre-registration queue) | Host (`main.rs`, `on_after_created`) | FIFO `VecDeque<String>` | `AppState.pending_window_labels` (Mutex) | `open_new_window`, `open_window_at_position`, `spawn_pool_window` (push), `on_after_created` (pop) | No cross-process sync; internal CEF ordering gate |
| Backend process handle & PID | Host | `Option<std::process::Child>` + `Option<u32>` | `AppState.sidecar_child`, `AppState.backend_pid` (Mutex) | `sidecar::spawn_backend()` on startup, `main.rs:369-373` on shutdown | None; fire-and-kill. Backend tracks host PID via parent-watcher; no heartbeat |
| Job Object handle (Windows process group) | Host | `JobHandle` (Windows `HANDLE` wrapper, Send+Sync) | `AppState.job_handle` (Mutex, Windows-only) | `sidecar::spawn_backend()` (`sidecar.rs:~130`) | Implicit; OS enforces `KILL_ON_JOB_CLOSE` when host exits |
| Backend endpoints (WS + web URLs) | Host | `BackendEndpoints` `{ws_endpoint, web_endpoint}` | `AppState.backend_endpoints` (Mutex) | `sidecar::spawn_backend()` on ready (`main.rs:234-237`) | Injected into frontend page load via IPC query in init |
| Frontend-visible window list | Frontend (`app-init.ts`) | SolidJS Signal `string[]` | `openWindowLabelsAtom` (`store/global.ts:138`) | `initInstanceTracking()` polls `listWindows()` every 50ms until list changes (`app-init.ts:~120-160`) | `window-instances-changed` event triggers poll; polling adds 100-300ms observer lag |
| Window instance count | Frontend | SolidJS Signal (derived from atom length) | `openWindowLabelsAtom.length` read by `InstancePanel` | Frontend poll (above) | Stale during spawn → ready gap (~100ms); `InstancePanel` uses array.length directly |
| Drag session state | Host (`state.rs`) | `Option<DragSession>` `{drag_id, type, source_window, tab_id, workspace_ids, payload}` | `AppState.active_drag` (Mutex) | Tear-off IPC handler (`drag.rs`), `tear_off_hook.rs` on finalize | Implicit; `tearoff:finalize` event carries final state to source renderer |
| Browser panes (embedded browsers) | Host | Managed by `BrowserPaneManager` (`crate::browser_panes`) | `AppState.browser_panes` | Pane spawn/close via backend APIs | Pane lifecycle independent from window lifecycle (cross-HWND child embed) |
| Backend-to-frontend window mapping | Host (`client.rs`) | `HashMap<String, String>` (host label → backend window_id) | `AppState.window_id_map` (Mutex) | Populated by frontend `register_backend_window()` IPC on init, cleared on close (`client.rs:308`) | IPC two-way: host looks up ID on close, backend cleanup is async fire-and-forget |
| Launcher single-instance enforcement | **Host** (not the launcher exe) | IPC port file + TCP port binding | `<data-dir>/cef/ipc-port` file (`port:token`) | Host on startup checks existing port → sends `open_new_window` → exits. Fresh instance writes file (`agentmux-cef/main.rs:187-212, 356-359`) | Stale file not cleaned on hard crash; defender: 2s TCP timeout (`main.rs:193`). Per-data-dir, so different versions/portables don't collide. |
| Backend DB state (Client, Window, Workspace, Tab objects) | Srv (`wcore.rs`) | SQLite via `WaveStore` (persistent) | `AppState.wstore` (`Arc<WaveStore>`) | Backend service methods (`WindowService`, `WorkspaceService`, etc.) | Srv owns authoritative state; host reflects via IPC queries + WPS events (WebSocket) |
| Agent process tracking | Srv (`process_tracker.rs`) | Registry of active agent PTY processes per block | `AgentProcessRegistry` (global via `set_global`) | Frontend block spawn, backend agent lifecycle | Delta events: `agent:process-added`, `agent:process-exited` broadcast to frontend |

---

## Process Model

```
┌─────────────────────────────────────────────────────────────────────┐
│ LAUNCHER (agentmux-launcher) — DLL search path wrapper only        │
│  • SetDllDirectoryW(runtime/) so libcef.dll is found               │
│  • Spawns runtime/agentmux-<version>.exe, waits, forwards exit    │
│  • As of this analysis: NO IPC, NO single-instance check, NO Job  │
│    Object. Just a thin spawn wrapper.                              │
└────────────────┬────────────────────────────────────────────────────┘
                 │ spawn & wait
                 │
┌────────────────▼────────────────────────────────────────────────────┐
│ HOST (agentmux-cef) — BROWSER PROCESS                              │
│                                                                     │
│  AppState:                                                          │
│   • browsers: {label → CEF Browser}                                │
│   • window_instance_registry: {label → instance#}                 │
│   • window_pool: FIFO queue of prewarmed labels                   │
│   • unpromoted_pool_labels: HashSet of ~100ms-old spawns          │
│   • sidecar_child: std::process::Child to srv                     │
│   • backend_endpoints: {ws, web URLs from srv}                    │
│   • job_handle: (Windows) Job object keeping srv alive             │
│                                                                     │
│  Single-instance check (main.rs:187-212):                           │
│   • Read <data-dir>/cef/ipc-port; if connectable, send             │
│     open_new_window via HTTP POST and exit 0.                      │
│   • Else: write port file with own ipc_port + ipc_token, continue. │
│                                                                     │
│  Key events:                                                        │
│   • on_after_created: register browser, init pool (if "main")      │
│   • on_before_close: unregister, cascade-close subwindows,        │
│     cleanup pool window                                             │
│   • pool:promote (from frontend): promote_pool_window() →          │
│     SetWindowPos, show, emit event, refill                         │
│                                                                     │
│  Shutdown: all browsers close → CEF quits message loop →           │
│    child.kill() sidecar (fire-and-forget) → shutdown() CEF →      │
│    delete port file → exit                                         │
└────────────────┬────────────────────────────────────────────────────┘
                 │ spawn (blocking on AGENTMUXSRV-ESTART)
                 │
┌────────────────▼────────────────────────────────────────────────────┐
│ SIDECAR (agentmux-srv) — BACKEND SERVICE                           │
│                                                                     │
│  AppState:                                                          │
│   • wstore: SQLite Client/Window/Workspace/Tab objects             │
│   • broker: event bus for WPS (WebSocket protocol stream)          │
│   • event_bus: EventBus for frontend broadcasts                    │
│   • process_tracker: registry of agent PTY processes                │
│                                                                     │
│  Lifecycle:                                                         │
│   • Parent watcher (Linux/macOS kqueue/pidfd, Windows Job Object)  │
│   • Listens on dynamic TCP ports (web + ws, separate)              │
│   • Auto-exits when parent (host) dies                             │
│   • Auto-archives idle sessions (>7 days, 2GB cap)                │
│                                                                     │
│  No explicit window/instance tracking — tracks Clients + Blocks    │
│  only. Window count inferred from host via list_windows IPC.       │
└────────────────┬────────────────────────────────────────────────────┘
                 │ web: /, ws: / (WPS)
                 │
┌────────────────▼────────────────────────────────────────────────────┐
│ FRONTEND (TypeScript/SolidJS in CEF renderer process)              │
│                                                                     │
│  Global state (store/global.ts):                                   │
│   • clientId, windowId, staticTabId (set once at init)             │
│   • openWindowLabelsAtom: SolidJS Signal (polled list)             │
│   • windowCountAtom: derived from atom length                      │
│   • Derived: workspace, activeTabId, waveWindow (from WOS)         │
│                                                                     │
│  Initialization (app-init.ts):                                     │
│   • If ?pool=1: await pool:promote event (with workspace ID)       │
│   • Else: call initHostNewWindow(workspaceId from ?workspace=)     │
│   • Listen for window-instances-changed → refetch labels           │
│   • Poll list_windows() every 50ms if count changed detected       │
│                                                                     │
│  InstancePanel (statusbar):                                         │
│   • Reads openWindowLabelsAtom, filters (main + window-*)          │
│   • Calls focusWindow(label) IPC → host → SetForegroundWindow      │
│   • Calls openNewWindow() IPC → cold-path window creation          │
│                                                                     │
│  Pool promotion (app/init/pool.ts):                                │
│   • awaitPoolPromote(): wait for pool:promote event                │
│   • On receive: push workspaceId into URL, signal pool_window_ready │
│     (must signal AFTER listener installed to avoid race)           │
│                                                                     │
│  Tearoff (tabbar-dnd.ts + drag.rs):                               │
│   • Frontend: emit tearoff start → starts SC_MOVE modal loop       │
│   • Host tearoff-hook tracks mouse over windows, emits hover events │
│   • On drop: backend MoveTabToWorkspace + frontend closes window   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Lifecycle Flows

### (a) Cold Launch

1. Launcher starts; spawns host, waits.
2. Host initializes CEF, creates `AppState`.
3. Host checks port file → none found (first run).
4. Host spawns sidecar (blocks until AGENTMUXSRV-ESTART).
5. Host starts IPC server, writes port file (`port:token`).
6. Host creates "main" browser → `on_after_created` fires.
7. `on_after_created`: registers "main", calls `init_pool()` → spawns 2 pool windows at offscreen coords with `?pool=1` flags.
8. Each pool window's `on_after_created` → `register_pool_window()` → hides from taskbar, waits for renderer.
9. Frontend (main window) loads, detects no `?pool=1`, runs `initHostNewWindow()` normally.
10. Frontend broadcasts `window-instances-changed` event (count=1, host doesn't count pool).
11. Frontend polls `list_windows()` → gets `["main"]` (pool windows filtered by `unpromoted_pool_labels`).
12. Pool windows' renderers signal `pool_window_ready` → `mark_pool_window_renderer_ready()` → moves label to queue, tries to refill.
13. **Final state:** host has 3 top-level HWNDs (1 real + 2 hidden pool), frontend sees only 1.

### (b) Opening a 2nd Window

1. User clicks "Open another window" in `InstancePanel`.
2. Frontend calls `openNewWindow()` IPC.
3. Host path (`open_new_window` handler, `drag.rs`):
   - Checks pool: if `window_pool.pop_front()` succeeds → warm path
   - Else: cold path → `open_window_at_position()`
4. **Warm path (pool available):**
   - `promote_pool_window()` pops label, removes from `unpromoted_pool_labels`
   - Calls `set_taskbar_hidden(hwnd, false)` → clears `WS_EX_TOOLWINDOW`
   - `SetWindowPos` to cursor position, `ShowWindow(SW_SHOW)`
   - Registers label in `window_instance_registry` → increments instance count
   - Broadcasts `window-instances-changed` event (count=2)
   - Emits `pool:promote` event to the promoted window's renderer with workspaceId
   - Calls `spawn_pool_window()` to refill queue
5. **Cold path (no pool):**
   - Calls `post_create_window()` → posts UI task to CEF
   - Pushes label onto `pending_window_labels` queue
   - Waits for `on_after_created` to pop it (timing gate)
   - Creates browser → `on_after_created` registers, broadcasts event
   - Caller then calls `post_window_at_position()` to move HWND (race window!)
6. Frontend receives `window-instances-changed` (count=2).
7. Frontend polls `list_windows()` → now gets `["main", "window-{uuid}"]`.
8. `InstancePanel` recomputes: "2 windows".

### (c) Tearing Off a Tab

1. User drags tab across title bar.
2. Frontend emits IPC `tear_off_start` with `{tab_id, source_workspace_id, source_window_label, cursor_x, cursor_y}`.
3. Host IPC handler `tear_off_start`:
   - Calls `crate::commands::drag::tear_off_sc_move_handshake()` → warm or cold path.
   - **Warm path:** calls `promote_pool_window()` with workspace ID → moves window to cursor, shows, emits promote.
   - **Cold path:** calls `open_window_at_position()` with cursor coords, then `post_window_at_position()` to move it.
   - Installs low-level mouse hook to track cursor until mouseup.
4. Host low-level hook (`tear_off_hook.rs`):
   - On each `WM_MOUSEMOVE`: `WindowFromPoint()` → checks if over another AgentMux window's tab strip.
   - Emits `tearoff:hover-changed` to candidate window.
   - On `WM_LBUTTONUP`: emits `tearoff:finalize` with `{source_label, dest_label, tab_id, workspaceId}`.
5. Frontend in destination window's renderer:
   - Receives `tearoff:finalize`.
   - Calls backend `MoveTabToWorkspace(tab_id, workspaceId)`.
   - Emits close signal to source window.
6. Source window renderer (now empty):
   - Closes itself via `closeWindow()` IPC → host closes browser via `PostMessageW(WM_CLOSE)`.
7. Host `on_before_close` for source window:
   - Unregisters from `browsers` map.
   - Calls `window_instance_registry.unregister(label)`.
   - Broadcasts `window-instances-changed` (count=2 → count=1).
   - If label was pool window: cleanup via `on_pool_window_destroyed()`.
8. Frontend (remaining) receives event, polls, updates `InstancePanel`.

### (d) Closing One Window (of 2+)

1. User closes a non-main window.
2. Frontend close handler calls `closeWindow({ label })` IPC.
3. Host `close_window_by_label()`:
   - Win32: posts `WM_CLOSE` to HWND.
   - Non-Win32: posts UI task to call `window.close()`.
4. CEF delivers `on_before_close`:
   - Unregisters from `browsers` map.
   - Calls `window_instance_registry.unregister()`.
   - Looks up backend `window_id`, async-notifies backend (fire-and-forget).
   - Broadcasts `window-instances-changed` (count=2 → count=1).
   - If label was pool window: cleanup (release semaphore, refill if needed).
5. Frontend polls, updates `InstancePanel`.
6. Backend async task eventually syncs (may lag).

### (e) Closing the LAST Window (Full App Exit)

1. User closes the final visible window.
2. Host `on_before_close`:
   - Unregisters browser.
   - **Special case:** checks if this was the last real instance.
     - Remaining: only pool windows in `browsers` + any browser-pane children.
     - If all remaining are pool or pane: no more instances → proceed with app exit.
   - Broadcasts `window-instances-changed` (count=1 → count=0).
3. CEF `on_before_close` returns false (allow close).
4. CEF fires `on_after_close` (final browser closed callback).
5. Host `client.rs`: checks `browser_list.len()` in `on_before_close` → `is_closing = true`.
6. When `browser_list` becomes empty: CEF exits message loop.
7. Host `main.rs` resumes after `run_message_loop()`:
   - Calls `child.kill()` on sidecar (fire-and-forget).
   - Calls `shutdown()` (CEF cleanup).
   - Deletes port file.
   - Exits process.
8. **Srv parent watcher:**
   - Linux: ppid changes (reparented to init) OR `pidfd_open` detects process exit.
   - macOS: kqueue `EVFILT_PROC` fires.
   - Windows: Job Object `KILL_ON_JOB_CLOSE` triggers.
   - Sidecar exits (may take 1-2s to notice on Linux polling).
9. Launcher (if any) receives exit code, forwards to OS.

---

## Gaps & Contradictions

### 1. Pool window instance count inflation

- `promote_pool_window()` calls `window_instance_registry.register(label)` → increments `next_num` (`window_pool.rs:484`).
- But `spawn_pool_window()` does NOT register in the registry; only adds to `unpromoted_pool_labels` (`window_pool.rs:86-88`).
- **Contradiction:** if a pool window is destroyed before promote (renderer crash), `on_pool_window_destroyed()` calls `window_instance_registry.lock().unregister()` BUT the label was never registered in the first place → silent no-op, no refund of instance#.
- Frontend's `openWindowLabelsAtom` is correctly filtered (uses `list_windows()` which filters `unpromoted_pool_labels`), but if a pool window crashes mid-spawn and the semaphore isn't released, next tear-off will cold-path → creates instance# 3 instead of reusing 2.
- **Symptom:** `InstancePanel` shows "Window 3" after a tear-off crash.
- **Root cause:** pool windows should NOT pre-register in instance registry; only on promote (already happens in `promote_pool_window:484`, but `on_pool_window_destroyed:307` tries to unregister non-existent entry).

### 2. Taskbar entry flicker during pool window promote (Phase 6 incomplete)

- Pool windows spawn hidden with `WS_EX_TOOLWINDOW` set (`register_pool_window:181`).
- On promote, `set_taskbar_hidden(hwnd, false)` clears the flag (`window_pool.rs:440`).
- Win32 requires hide→show cycle to refresh taskbar (`set_taskbar_hidden:224-225`).
- **Gap:** the promote path sets taskbar visibility but does NOT re-show the window before the subsequent `ShowWindow(SW_SHOW)` — the order is: clear toolwindow flag → `SetWindowPos` (`SWP_FRAMECHANGED`, no show) → then `ShowWindow(SW_SHOW)`. The hide/show inside `set_taskbar_hidden(false)` should NOT happen for pool windows (comment says "Don't re-show pool windows" line 233), BUT promote needs that cycle to take effect.
- **Symptom:** promoted window may not appear in taskbar until user interaction.
- **Root cause:** phase6 incomplete; promote path should do: hide → clear flag → `SetWindowPos` → **re-show** (not deferred).

### 3. Stale launcher port file after hard crash (Windows)

- Host checks port file; if stale (process died without cleanup), `TcpStream::connect_timeout(2s)` fails.
- **Defense:** treats timeout as stale → continues with fresh launch (`main.rs:207-208`).
- **Gap:** if previous host died during `run_message_loop()` (pre-CEF shutdown), sidecar may still be alive.
- Previous host calls `child.kill()` but does NOT wait (fire-and-forget); sidecar takes 1-2s to notice death on Linux.
- **New launcher:** spawns new host → new sidecar → but OLD sidecar still running on same dynamic port ranges.
- **Symptom:** "taskbar entry but no window" — new host can't find port, creates fresh sidecar on *different* port, but old sidecar (listening on old web port) is still alive with old Client/Window/Tab objects in DB.
- On next host restart: host re-opens DB, sees old objects, frontend confused by dangling references.
- **Root cause:** no explicit sidecar shutdown wait; fire-and-forget `child.kill()` doesn't guarantee sidecar has exited before launcher starts next one.

### 4. Frontend-backend state divergence on tab/workspace changes

- Frontend tracks `activeTabId` via `workspace.activetabid` (Jotai memo, now SolidJS memo).
- Backend authoritative (WaveStore SQLite).
- **Gap:** no explicit sync on workspace update; relies on WPS event flow (WebSocket push from backend).
- If WPS connection drops mid-tab-switch: frontend's memo may lag server state by seconds.
- **Symptom:** user switches tabs in one window, sees outdated tab content in `InstancePanel` or other window.
- **Root cause:** WPS is best-effort; no handshake / ack per state mutation.

### 5. Window instance registry mismatch during pool lifecycle

- `WindowInstanceRegistry` is in-memory only (`state.rs:44-81`), NOT persisted.
- On pool promote: `window_instance_registry.register(label)` assigns instance# (`window_pool.rs:484`).
- But pool window destroy before promote: tries to `unregister()` a label that was never `register()`-ed (`client.rs:307`).
- **Symptom:** silent no-op; next new window reuses the same instance# if a pool window crashed.
- Real issue: pool windows should have a flag "promoted=false" so destroy doesn't try to unregister.
- **Root cause:** architectural — unpromoted pool windows should NEVER be in instance registry until promote fires.

### 6. Missing cleanup for browser-pane windows on app exit

- `browser-pane-*` labels are child HWNDs (embedded CEF views), not top-level windows.
- On app exit: main window closes → `on_before_close` fires → main loop quits.
- But `browser_panes` manager may still hold refs to live browser-pane windows.
- CEF should clean them up as part of `shutdown()`, but no explicit order guarantee.
- **Symptom:** rare orphan pane HWND after exit (usually not visible, but memory leak if rapid on/off).
- **Root cause:** pane lifecycle not explicitly tied to main window close.

### 7. Pool window refill race during bursty tear-offs

- `window_pool_respawn_in_flight: AtomicBool` serializes spawns (`window_pool.rs:239-241`).
- On tear-off: `promote_pool_window()` pops, then calls `spawn_pool_window()` to refill.
- If user tears off 2 windows in <100ms (time for CEF to spawn + render-ready signal):
  - 1st tear-off: promote pops `pool[0]`, sets semaphore=true, calls spawn.
  - 2nd tear-off: tries to promote `pool[1]` (succeeds), calls spawn.
  - 1st spawn finishes: calls `mark_pool_window_renderer_ready()` → sets semaphore=false.
  - 2nd spawn's render-ready races; if late, semaphore is already false.
- **Symptom:** burst tear-offs can empty the pool entirely if timing is unlucky.
- **Root cause:** semaphore SWAPPED to true in `spawn_pool_window()`, not held for duration; released on renderer-ready, not spawn-complete. If second spawn fires while first is in-flight, it checks the OLD semaphore state.

### 8. "No window on relaunch" symptom root

- Previous app exit: all windows close, `sidecar_child.kill()` fires (async).
- Port file cleanup is deferred until `main.rs:383` (after shutdown + runtime drop).
- **If user launches immediately (within 100ms):**
  - Port file still exists (host didn't get to cleanup yet).
  - Launcher connects, sends `open_new_window` IPC → but old host's IPC server is dead (CEF message loop exited).
  - IPC request is ignored or fails silently.
  - Launcher exits (success code).
  - User sees no window because new instance never started.
- **Root cause:** port file cleanup happens AFTER CEF shutdown is fully complete, but launcher can connect to stale port file before cleanup. **Aggravating cause** (current zombie scenario): if host hangs because pool windows keep CEF's message loop alive, port file is never cleaned and IPC server appears alive, so this path stays broken forever.

---

## Architectural Smells

1. **Ad-hoc HashMap with no invariants** (`browsers`, `window_meta`, `window_id_map`)
   - `browsers: HashMap<String, Browser>` with parallel `window_meta: HashMap<String, WindowMeta>`.
   - No invariant: every entry in `browsers` should have a corresponding `window_meta` (eventually fixed in `on_after_created:132-140`, but not enforced).
   - Risk: code paths that add/remove from one but not the other → use-after-free or stale metadata.

2. **Fire-and-forget IPC with no ack**
   - Sidecar `child.kill()` is async, no wait.
   - Backend window cleanup from `on_before_close` is async, no ack back to host.
   - Risk: frontend sees window closed before backend has synced; backend still has refs to deleted tabs.

3. **Locks held across async boundaries**
   - `AppState` fields are `Mutex` (parking_lot, non-async-aware).
   - IPC handlers (async Tokio threads) lock `state.browsers.lock()` → does file I/O / WPS send inside lock → can hold lock for milliseconds.
   - Risk: CEF UI thread trying to call `on_after_created` blocks on `browsers.lock()`, freezing UI.

4. **Polling instead of event-driven window tracking**
   - Frontend `initInstanceTracking()` polls `list_windows()` every 50ms on state change, up to 6× (`app-init.ts:130-160`).
   - Only triggered AFTER `window-instances-changed` event (count), not label change.
   - Backend has no way to signal "new window X created" — only count broadcast.
   - Risk: high-frequency jitter in `InstancePanel`; 100-300ms observer latency.

5. **Instance count from two different sources with no sync**
   - Host broadcast: `window_instance_registry.count()` (call site: `window_pool.rs:493`).
   - Frontend displayed: `openWindowLabelsAtom.length` (filtered `list_windows` result).
   - They CAN diverge: pool windows alive in `browsers` but not yet promoted, orphan browser-pane children.
   - Risk: `InstancePanel` shows "2 windows" but `window_instance_registry.count()` is 3.

6. **State mutation order dependency**
   - `on_before_close` must:
     1. Unregister from `browsers` map.
     2. Call `window_instance_registry.unregister()`.
     3. Emit event.
     4. Notify backend.
   - If step 2 is skipped for pool windows (should be), but step 1 happens, backends later see mismatches.
   - Risk: out-of-order steps cause silent corruption.

7. **No explicit lifecycle for sidecar**
   - Sidecar tracks parent via ppid (Linux) or Job Object (Windows).
   - No explicit "shutdown" RPC to backend; just kill + hope.
   - Risk: sidecar can exit uncleanly (OOM, panic, signal) and leave temp files / database locks.

8. **Pool window label prefix collision potential**
   - Pool windows: `"window-pool-{uuid}"`.
   - Regular windows: `"window-{uuid}"`.
   - Code filters by `label.starts_with("window-pool-")` in multiple places.
   - Risk: if a regular window label ever starts with `"window-pool-"` (collision), it will be treated as pool.

---

## Concrete Symptom Mapping

| Observed Symptom | Root Cause | Evidence |
|---|---|---|
| **Zombie processes after all windows close** (launcher + multiple host renderers + srv) | Host hangs because pool windows keep CEF's message loop alive (gap #2 incomplete). Host never exits → its srv-job never closes → srv lives. Launcher is also blocked on `child.wait()`. | `main.rs:371-372`, `agentmux-srv/main.rs:28-44` |
| **Taskbar entry, no window on relaunch** | Stale port file + hung host's IPC server still accepting `open_new_window` but doing nothing (gap #8) | `main.rs:187-208` (2s timeout), `:382-383` (deferred cleanup) |
| **Pool windows leak to taskbar** | Incomplete Phase 6 — promote path doesn't re-show after clearing `WS_EX_TOOLWINDOW` (gap #2) | `window_pool.rs:440`, `set_taskbar_hidden:233` comment |
| **Instance count inflation after tear-off crash** | Pool window destroy tries to unregister non-existent registry entry (gap #5) + semaphore not released (gap #7) | `window_pool.rs:245-274`, `client.rs:307` |
| **InstancePanel flickers or shows wrong count** | Frontend polls with lag; instance registry ≠ label list (gap #5) | `app-init.ts:130-160` (6×50ms polling) |
| **Tab visible in one window, missing in another** | WPS event delivery lag + no ack (smell #2) | `store/global.ts:66-75` (memo-based, best-effort) |
| **Crash on click "Open another window"** | Lock held during file I/O in `on_after_created` (smell #3) | `client.rs:97-205` (`browsers.lock` held through callbacks) |

---

## File Reference Index

- Core host state: `agentmux-cef/src/state.rs:41-281`
- Window lifecycle: `agentmux-cef/src/client.rs:97-350`
- Pool model: `agentmux-cef/src/commands/window_pool.rs:1-525`
- Launcher: `agentmux-launcher/src/main.rs:1-158`
- Sidecar spawn / Job Object: `agentmux-cef/src/sidecar.rs` (`create_job_object_for_child` at `:424`)
- Sidecar lifecycle: `agentmux-srv/src/main.rs:28-236`
- Frontend poll: `frontend/app-init.ts:~120-160`
- Pool frontend: `frontend/app/init/pool.ts:17-75`
- Instance panel: `frontend/app/statusbar/InstancePanel.tsx:1-180`
