# Why does AgentMux need a "host should quit" saga?

**Status:** Architectural analysis. Written 2026-04-29 after multiple smoke iterations on B.9.3.
**Author:** AgentA.
**Goal:** zoom out far enough that the design choices become obvious. Diagrams from launch → tab tear → close → quit.

---

## The core invariant

> When the user closes every visible AgentMux window, the host process tree should exit.

This is the basic user contract. They closed everything, the app should be gone. CPU should be free, memory should be free, the taskbar should be clean.

**Why this is non-trivial in our architecture**: AgentMux is not "one process per window". The host process owns multiple CEF browsers — visible top-level windows AND a hidden warm pool of pre-spawned browsers (so tear-off is instantaneous). The OS has no concept of "user-meaningful windows vs hidden infrastructure". When all visible windows close, the host process is still busy maintaining the pool, so it doesn't exit on its own.

Something has to **decide** that the user is done and tell the host process to terminate. That decision needs to be:

1. **Correct**: only fire when the user is truly done (not when they're between windows).
2. **Reliable**: fire deterministically, not based on heuristics or timers.
3. **Cross-platform**: same logic works on Windows / macOS / Linux.

---

## The full lifecycle

### 1. Launch

```
USER double-clicks agentmux.exe
       │
       ▼
LAUNCHER (one process)
       │ creates J0 Job Object (KILL_ON_JOB_CLOSE)
       │ binds \\.\pipe\agentmux-{hash}\command (single-instance lock)
       │ spawns IPC server on tokio runtime  ←─── reducer lives here
       │ spawns srv (assigned to J0)
       │ spawns host (assigned to J0)
       │
       ▼
HOST process boots
       │ CEF init: creates main process + helper subprocesses
       │ launcher_ipc connects to pipe → Register → reducer marks Host=Running
       │ WRR install_hooks: captures UI thread id, SetWinEventHook installed
       │ creates "main" CEF Browser
       │   → on_after_created fires:
       │       state.browsers["main"] = Browser
       │       launcher_ipc.report_window_opened("main", FullInstance, None)
       │   → launcher reducer:
       │       state.windows["main"] = WindowMirror{...}
       │       Event::WindowOpened broadcast to subscribers
       │ pool warming: spawn 3 hidden pool windows
       │   for each: state.browsers["window-pool-N"] = Browser
       │             state.unpromoted_pool_labels.insert("window-pool-N")
       │             launcher_ipc.report_pool_window_added(...)
       │
       ▼
RUN_MESSAGE_LOOP (blocking call on host's main thread)
       │ This thread is now "the UI thread" — what CEF docs call
       │ "the main application thread". From here forward:
       │   - cef::quit_message_loop must be called HERE
       │   - on_before_close, on_after_created, etc., all run HERE
       │   - cef::post_task(TID_UI, ...) targets THIS thread
```

State after launch is steady:

```
LAUNCHER reducer state              HOST AppState
state.windows = { "main" }          state.browsers = {
state.pool = {                          "main": Browser,
    "window-pool-1",                    "window-pool-1": Browser,
    "window-pool-2",                    "window-pool-2": Browser,
    "window-pool-3",                    "window-pool-3": Browser,
}                                   }
state.processes = { Host: Running   state.unpromoted_pool_labels = {
                  , Srv:  Running       "window-pool-1",
                  }                     "window-pool-2",
                                        "window-pool-3",
                                    }
```

### 2. Tab tear-off

User drags a tab out of the main window past the threshold. The host's `commands::drag::tear_off` runs on the UI thread:

```
                user finishes drag
                       │
                       ▼
HOST (UI thread):
  promote_pool_window("workspace_id", x, y)
       │
       │ 1. pop "window-pool-1" from pool queue
       │ 2. unpromoted_pool_labels.remove("window-pool-1")
       │ 3. rename to "window-{uuid}" — but: state.browsers KEY stays
       │    "window-pool-1" until on_after_created creates a new entry?
       │    (this is the part that's actually delicate; see real source)
       │ 4. position the window where the user dropped
       │ 5. spawn a new pool window to refill: "window-pool-N+1"
       │
       ▼ (per CEF Views creating a fresh top-level)
  on_after_created fires for the freshly-promoted window:
       │ state.browsers["window-{uuid}"] = Browser
       │ launcher_ipc.report_pool_window_removed("window-pool-1")
       │ launcher_ipc.report_window_opened("window-{uuid}", FullInstance, None)
       │
       ▼
LAUNCHER reducer:
  state.pool.remove("window-pool-1")
  state.windows.insert("window-{uuid}", ...)
  Event::PoolWindowRemoved + Event::WindowOpened broadcast
```

After one tear-off:

```
state.windows = { "main", "window-abc123..." }
state.pool = { "window-pool-2", "window-pool-3", "window-pool-4" }  ← refilled
state.browsers = { "main", "window-abc123...", "window-pool-2", ..., "window-pool-4" }
```

### 3. Close all visible windows — what's SUPPOSED to happen

User clicks X on the torn-off window first, then on main.

```
3a. Close torn-off window
────────────────────────
  user clicks X
       │
       ▼
CEF lifecycle (UI thread):
  on_before_close("window-abc123...") fires
       │
       │ host's client.rs::on_before_close:
       │   browsers.remove("window-abc123...")
       │   launcher_ipc.report_window_closed("window-abc123...")
       │
       │   ── compute user_browser_count ──────────────
       │   filter state.browsers excluding:
       │     - keys in unpromoted_pool_labels
       │     - keys starting with "browser-pane-"
       │   = { "main" } → count = 1
       │   ────────────────────────────────────────────
       │
       │   if count == 0 && !is_pane → quit_message_loop()
       │   ELSE → emit window-instances-changed, continue
       │
       ▼
LAUNCHER reducer:
  state.windows.remove("window-abc123...")
  state.windows = { "main" }                  ← still has main
  Event::WindowClosed broadcast
  (B.9.3) check: state.windows.is_empty()? NO → no saga

3b. Close main window
─────────────────────
  user clicks X on main
       │
       ▼
CEF lifecycle (UI thread):
  on_before_close("main") fires
       │
       │ host's client.rs::on_before_close:
       │   browsers.remove("main")
       │   launcher_ipc.report_window_closed("main")
       │
       │   ── compute user_browser_count ──────────────
       │   filter state.browsers excluding:
       │     - keys in unpromoted_pool_labels
       │     - keys starting with "browser-pane-"
       │   browsers = { "window-pool-2", "window-pool-3", "window-pool-4" }
       │   pool      = { "window-pool-2", "window-pool-3", "window-pool-4" }
       │   = { } → count = 0 ✓
       │   ────────────────────────────────────────────
       │
       │   if count == 0 && !is_pane:
       │     quit_message_loop()  ✓ ← CALLED HERE on UI thread
       │
       ▼
CEF run_message_loop sees the quit signal, returns
       │
       ▼
HOST main() falls out of run_message_loop, drops handles, exits
       │
       ▼
J0's KILL_ON_JOB_CLOSE reaps srv + render workers
       │
       ▼
Tree exits ✓
```

**This is the design. It works on paper. It does NOT work in our smoke.**

### 4. What's actually happening — the bug

Smoke evidence from v0.33.491–v0.33.494 (each: tear off + close all):

```
[host]  Unregistered browser: label=window-{tear-off} (remaining: 4)
[host]  Unregistered browser: label=main (remaining: 3)
[host]  ?? "last user-facing window closed (...) — calling quit_message_loop"
        ── this log line is MISSING ──

[launcher] reducer: state.windows.remove("window-{tear-off}") → still {main}
[launcher] reducer: state.windows.remove("main") → state.windows.is_empty() ✓
[launcher] (B.9.3) emits Event::HostShouldQuit
[host]  receives HostShouldQuit on tokio thread
[host]  ?? "QuitMessageLoopTask running on UI thread"
        ── this log line is MISSING (saga delivery failed) ──

[host process tree stays alive forever]
```

**Two independent bugs were uncovered, both labeled in `b9-3-quit-thread-analysis.md`:**

```
                               ┌──────────────────────────────────────┐
                               │  CLOSE-ALL TIMELINE (broken state)   │
                               └──────────────────────────────────────┘

main window closes
       │
       ▼
on_before_close("main") fires on UI thread
       │
       │ ┌─── CAUSE A ──────────────────────────────────────────────┐
       │ │ user_browser_count == 0 && !is_pane gate doesn't fire.  │
       │ │ Possible reasons (unverified):                           │
       │ │   (a) is_pane is true — closing browser is on a          │
       │ │       pane-client, gate's "main client only" skip kicks. │
       │ │   (b) pool refill races: a new "window-pool-N+1" lands   │
       │ │       in state.browsers before unpromoted_pool_labels    │
       │ │       is updated, so it counts as user-facing.           │
       │ │   (c) something else.                                    │
       │ │                                                          │
       │ │ Whichever it is, quit_message_loop is NOT called here.   │
       │ │ This is a host-local bug. Cross-platform — same code on  │
       │ │ Windows / macOS / Linux. One fix covers all.             │
       │ └──────────────────────────────────────────────────────────┘
       │
       ▼
launcher_ipc.report_window_closed("main") sent
       │
       ▼
LAUNCHER reducer (tokio thread):
       │ state.windows.remove("main") → empty
       │ B.9.3 transition check: empty? yes. host running? yes. → fire saga
       │
       │ Event::HostShouldQuit emitted
       │
       ▼
HOST receives Event::HostShouldQuit on its tokio thread
       │
       │ ┌─── CAUSE B ──────────────────────────────────────────────┐
       │ │ Need to deliver "call quit_message_loop" to UI thread,   │
       │ │ from the tokio thread that received the event.           │
       │ │                                                          │
       │ │ Tried & failed:                                          │
       │ │   v.491  cef::post_task(TID_UI, task) — task never runs. │
       │ │          Earlier post_tasks DID run; this one is dropped.│
       │ │   v.492  call quit_message_loop directly from tokio —    │
       │ │          UB per CEF docs; no-op in practice.             │
       │ │   v.493  minimal post_task with only quit_message_loop — │
       │ │          still dropped. Confirmed Cause B is post_task,  │
       │ │          not anything we did.                            │
       │ │   v.494  Win32 PostThreadMessage(WM_QUIT) — accepted by  │
       │ │          OS (ok=true) but CEF's run_message_loop on      │
       │ │          Windows uses a custom pump that doesn't honor   │
       │ │          WM_QUIT to terminate. (Cross-platform: this     │
       │ │          workaround is Windows-only anyway.)             │
       │ │                                                          │
       │ │ Cross-platform implication: cef::post_task is the only   │
       │ │ portable abstraction for "deliver work to UI thread".    │
       │ │ If it's broken in this state, the bug is in CEF — not    │
       │ │ something we should paper over with three platform-      │
       │ │ specific bridges.                                        │
       │ └──────────────────────────────────────────────────────────┘
       │
       ▼
[no quit happens]
[host process tree stays alive forever]
```

---

## Why both paths exist (and why both should be fixed)

The reducer-driven saga (B.9.3) was added on the assumption that Cause A might never get diagnosed cheaply, and the launcher's view of "the user is done" is cleaner than the host's view (no pool / pane filtering needed — the launcher only ever sees user-meaningful labels via `report_window_opened`).

This is sound — the launcher's reducer is the canonical state. **But** the saga has to deliver work back to the host's UI thread to actually quit, and we ran into Cause B trying to do that.

So we have:

| Path | Detection | Delivery | Status |
|---|---|---|---|
| Host-local gate (pre-B.9) | `client.rs:515` count check | runs on UI thread already (in `on_before_close`) → direct call | **Cause A: detection broken** |
| Reducer-driven saga (B.9.3) | `apply_hwnd_destroyed` / `handle_report_window_closed` post-mutation check | tokio → ??? → UI thread | **Cause B: delivery broken** |

Both should work. Both eventually call the same `cef::quit_message_loop()` from the UI thread. The difference is who decides.

---

## Cross-platform implications

```
                         CROSS-PLATFORM PORTABILITY MATRIX

LAYER                              | Windows | macOS  | Linux  | Verdict
───────────────────────────────────|─────────|────────|────────|──────────
state.windows reducer mutation     |    ✓    |   ✓    |   ✓    | portable
Event::HostShouldQuit emission     |    ✓    |   ✓    |   ✓    | portable
host receives event on tokio thrd  |    ✓    |   ✓    |   ✓    | portable
client.rs:515 gate logic           |    ✓    |   ✓    |   ✓    | portable (Cause A)
cef::post_task delivery to UI      |    ✗    |   ?    |   ?    | CEF binding
cef::quit_message_loop on UI thrd  |    ✓    |   ✓    |   ✓    | portable
PostThreadMessage(WM_QUIT)         |    ✗    |   N/A  |   N/A  | win32 only,
                                   |         |        |        | doesn't work
Hidden message-only window         |    ?    |   N/A  |   N/A  | win32 only
WinEventHook (the WRR sensor)      |    ✓    |   N/A  |   N/A  | win32 only —
                                   |         |        |        | macOS/Linux
                                   |         |        |        | needs
                                   |         |        |        | NSWindow
                                   |         |        |        | / X11
                                   |         |        |        | equivalent
```

**Key insight**: WRR's *sensor* (the WinEventHook) is already Windows-only by necessity — there's no portable abstraction over OS window events. macOS / Linux ports will replace it with platform-specific equivalents.

But WRR's *signal* (the events flowing into the reducer) is portable. So is the saga emission. Only the **delivery** of the saga's resulting work back to the UI thread is platform-specific — and `cef::post_task` is supposed to abstract that.

If `cef::post_task` is unreliable, then either:
- It's a CEF bug — file upstream, accept current behavior, document the workaround per-platform.
- We're using it wrong — figure out the right idiom (e.g., post on a different thread first, or use post_delayed_task with 0).

The right thing is NOT to build three platform-specific bridges. That's how cross-platform code rots.

---

## Recommended path forward

**Primary fix — Cause A (cross-platform)**:
- Land v0.33.495's gate diagnostic.
- Run one smoke (tear off + close all) and read the `[wrr] app-exit gate: ...` log line.
- The diagnostic will print `user_count`, `is_pane`, the browsers map, and the unpromoted pool labels at the moment of decision. We will know IMMEDIATELY which of (a)/(b)/(c) is the culprit.
- Fix in client.rs (likely 1–5 lines). Cross-platform.

**Secondary — keep the saga, leave Cause B for later**:
- B.9.3's reducer arm + saga emission stays. They're cross-platform and useful even if the delivery is currently unreliable.
- Strip the v0.33.494 Win32 PostThreadMessage code (single-OS, doesn't work, dead end).
- Saga delivery: keep `cef::post_task`. When Cause A is fixed, the saga rarely fires; when it does, accept that it might be unreliable in the teardown window and treat it as a diagnostic / "should not be load-bearing" path.
- If we later determine `cef::post_task` is genuinely broken, file upstream + revisit with a CEF-binding-level fix that ALL platforms benefit from.

**What stays in the codebase**:
- WRR sensor (Windows-only, by design — macOS/Linux port adds equivalents)
- WRR reducer arms (cross-platform)
- `Event::HostShouldQuit` (cross-platform)
- `OrphanInstance` drift (cross-platform diagnostic, fires whenever Cause A regression-tests itself even if Cause A is fixed)

**What gets removed**:
- `wrr::win_event::post_thread_quit_message` (Win32 PostThreadMessage)
- `ui_tasks::post_quit_message_loop` if it was added (Win32-specific wrapper)
- TID capture in install_hooks (no longer needed)
- The `cfg(windows)` block in launcher_ipc that calls into post_thread_quit_message

---

## TL;DR diagram

```
                    THE BUG, IN ONE PICTURE
                    
        ┌─────────────────────────┐
        │  user closes everything │
        └──────────┬──────────────┘
                   │
                   ▼
       ┌─────────────────────┐
       │ on_before_close fires│
       │  on UI thread       │
       └──────────┬──────────┘
                  │
                  ▼
         ┌──────────────────┐         ┌────────────────────┐
         │  CAUSE A: gate   │   ──┐   │ launcher reducer   │
         │  silently fails  │     │   │ sees windows=∅      │
         │  to fire quit    │     │   └─────────┬──────────┘
         └──────────────────┘     │             │
                                  │             ▼
                                  │   ┌──────────────────┐
                                  │   │ saga:            │
                                  │   │ HostShouldQuit   │
                                  │   └─────────┬────────┘
                                  │             │
                                  │             ▼
                                  │   ┌──────────────────┐
                                  │   │ CAUSE B: delivery│
                                  │   │ to UI thread     │
                                  │   │ silently fails   │
                                  │   └──────────────────┘
                                  │
                                  ▼
                       ┌─────────────────────┐
                       │ host process        │
                       │ stays alive forever │
                       └─────────────────────┘

Fix Cause A (cross-platform, diagnostic data already in our pocket).
Saga stays as defense-in-depth (cross-platform).
Don't ship the Win32-only delivery hacks (single-OS, also doesn't work).
```
