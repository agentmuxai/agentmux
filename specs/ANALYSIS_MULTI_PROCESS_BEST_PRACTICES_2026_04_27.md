# ANALYSIS: Multi-Process Desktop App State Management — Best Practices

**Date:** 2026-04-27
**Author:** AgentC (via research subagent)
**Purpose:** Background research feeding
`SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`. Documents the prior
art the spec draws from, so reviewers can audit the reasoning rather
than take the recommendations on faith.

This is descriptive — *what mature codebases do* and *why* — not
prescriptive about AgentMux specifically (that's the spec's job).
References are inline.

---

## 1. Single-source-of-truth patterns for multi-process desktop apps

The mature consensus across Chromium, Electron, VS Code, and Tauri is
**the privileged native process owns the canonical window/tab list;
renderers/frontends hold a derived mirror**. This is not a stylistic
choice — it falls out of the fact that the native process is the only
one that holds OS handles (HWND, child-process handles, job-object
handles) whose existence is the ground truth.

- **Chromium** keeps a singleton `BrowserList` in the browser process
  (`chrome/browser/ui/browser_list.h`) plus per-window `Browser`
  objects, each of which owns a `TabStripModel`. Tabs are not mere
  data — `TabStripModel` owns each `WebContents`, observes its
  destruction through `WebContentsObserver::WebContentsDestroyed`, and
  notifies UI views via `TabStripModelObserver`. The renderer never
  holds the canonical list; it can't, because it doesn't even see
  other tabs. Modern Chromium is actively migrating away from the
  historical `Browser` "god object" toward `TabFeatures` /
  `TabInterface` ([Chromium tab helpers
  docs](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/tab_helpers.md),
  [chrome browser design
  principles](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/chrome_browser_design_principles.md)).

- **Electron** mirrors this exactly: `BrowserWindow` lives in the main
  process, and `BrowserWindow.getAllWindows()` is the authoritative
  list. Calls from a renderer (historically via `@electron/remote`) are
  *proxies* into the main process; `new BrowserWindow` from a renderer
  creates the window in the main process and returns a remote handle
  ([Electron BrowserWindow
  API](https://www.electronjs.org/docs/latest/api/browser-window),
  [Electron Process
  Model](https://www.electronjs.org/docs/latest/tutorial/process-model)).

- **VS Code** layers it: `IWindowsMainService` (primary windows) and
  `IAuxiliaryWindowsMainService` (torn-off floating windows) live in
  the Electron main process behind `NativeHostMainService`. Each
  `CodeWindow extends BaseWindow` wraps an `electron.BrowserWindow`;
  renderers reach the truth via IPC through `IHostService` →
  `INativeHostMainService` ([VS Code window
  management](https://deepwiki.com/microsoft/vscode/4.2-window-management-and-titlebar)).

- **Tauri** explicitly recommends "manage all state in Rust and tell
  the individual windows about changes" — Rust holds state via
  `Builder::manage()`, frontends mutate by `#[tauri::command]`, and
  updates fan out via `emit_all` / `emit_to` ([Tauri state
  management](https://v2.tauri.app/develop/state-management/),
  [Unifying state across frontend and backend in
  Tauri](https://medium.com/@ssamuel.sushant/unifying-state-across-frontend-and-backend-in-tauri-a-detailed-walkthrough-3b73076e912c)).

**Trade-off:** "Frontend owns truth, host follows" only works when the
frontend is single-process (e.g., a single SPA controlling one OS
window). With multiple windows + multiple render processes, no
renderer can see all peers, so it physically cannot be the source of
truth — it would have to round-trip through the host anyway, which is
what the established pattern already does with stricter ordering.

---

## 2. State-machine modelling of window lifecycle

Mature codebases model windows as observable states with explicit
transitions, not flags. Concrete prior art:

- **Chromium's `WebContents` + `TabStripModel`** uses an observer
  model and an internal "model index vs. view index" split during close
  animations — the window is "being closed but still tracked" (Mac
  TabStrip design notes the indices diverge during close-animation;
  the data structure stays alive until animation completes).
  `WebContentsObserver::WebContentsDestroyed` is the only authoritative
  "Closed" signal.

- **VS Code's `LifecycleMainService`** uses a numeric
  `LifecycleMainPhase` enum (`Starting=1`, `Ready=2`, `AfterWindowOpen=3`,
  `Eventually=4`) with `onBeforeShutdown` / `onWillShutdown`
  veto/extend hooks and an explicit `kill()` path
  ([microsoft/vscode #248693](https://github.com/microsoft/vscode/issues/248693),
  [#76679](https://github.com/microsoft/vscode/issues/76679)).

- **Elm Architecture** is explicitly a state machine: a finite
  `Model`, a `Msg` algebra, and a total `update : Msg -> Model -> Model`.
  The Elm community has formalized splitting `Msg` into **Intents**
  (commands) vs. **Facts** (events) — see ["Intents and Facts:
  pondering CQRS in
  Elm"](https://discourse.elm-lang.org/t/intents-and-facts-pondering-cqrs-in-elm/923)
  and
  [SAFE-ConfPlanner](https://github.com/SAFE-Stack/SAFE-ConfPlanner),
  which combines server-side CQRS/ES with client-side Elm.

A canonical lifecycle for an app like AgentMux:

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

Observable transitions (must be broadcast): `→Spawning`,
`→ReadyHidden`, `→Visible`, `→Closing`, `→Closed`, `Crashed`.
Internal-only: pre-`Spawning` queue position, internal handle re-bind
during promotion.

**Modelling pool/warm windows:** the dominant pattern (Chromium's
`SpareRenderProcessHostManager`) is a *separate type*, not a flag —
Chromium owns at most one spare renderer in a dedicated manager and
hands it over via `MaybeTakeSpareRenderProcessHost`, discarding it if
the requested `BrowserContext` / `StoragePartition` doesn't match
([SpareRenderProcessHostManager
source](https://chromium.googlesource.com/chromium/src/+/11c23f555a312563b67f67ae6e41f1e32fd73f1c/content/browser/renderer_host/render_process_host_impl.cc),
[crrev 2929113002](https://codereview.chromium.org/2929113002)). The
pool item enters the canonical list **only at the promotion
transition**, not at spawn — that single rule prevents pool windows
from inflating instance counts or appearing in "all windows"
iterations.

---

## 3. Reliable cross-process state sync (Rust truth ↔ React mirror)

The pattern that survives in production:

- **CQRS-lite split:** frontend sends *commands* (intents) via typed
  RPC (`#[tauri::command]`-style); backend emits *events* (facts) via
  a broadcast topic. SAFE-ConfPlanner is the canonical Elm/CQRS
  reference example.

- **Versioned snapshots + delta stream:** every event carries a
  monotonic `state_version`. On connect/reconnect, frontend requests
  `GetSnapshot()` returning `(version_v, full_state)` and starts
  applying deltas with `version > v`; if a delta arrives with a gap,
  it re-snapshots. Chromium's Mojo bindings formalize this with
  `[MinVersion=N]`, `[Stable]`, and `[Extensible]` attributes for
  forward/backward compatibility ([Mojo IDL
  docs](https://chromium.googlesource.com/chromium/src/+/master/mojo/public/tools/bindings/README.md)).
  Critical caveat: Mojo guarantees ordering *only within a single
  message pipe*, so a single stream per subscriber is essential.

- **Echo-loop guard:** when applying a remote-originated update
  locally, set a flag so the local store doesn't re-broadcast — the
  explicit recommendation in [gethopp.app's Tauri Zustand sync
  writeup](https://www.gethopp.app/blog/tauri-window-state-sync).

- **Rust event-sourcing crates**
  ([cqrs-es](https://crates.io/crates/cqrs-es),
  [esrs](https://github.com/primait/event_sourcing.rs)) provide the
  aggregate/event/projection model; for an app of this shape, the
  "aggregate" is the Window and the projection is the Redux store.

**Common bugs:** lost events on reconnect (no resync protocol),
interleaved deltas across separate pipes (no global ordering), stale
projections if events fire before commit (Salesforce calls this
"publish after commit"), and renderer-originated optimistic updates
that diverge when the backend rejects ([Service Broker
fire-and-forget
anti-pattern](https://davewentzel.com/content/service-broker-demystified-fire-and-forget-anti-pattern/),
[Salesforce remote process invocation
patterns](https://developer.salesforce.com/docs/atlas.en-us.integration_patterns_and_practices.meta/integration_patterns_and_practices/integ_pat_remote_process_invocation_fire_forget.htm)).

---

## 4. Process-tree lifecycle on Windows specifically

- **Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`** are the
  only reliable Windows-native cleanup mechanism. Create the job, set
  `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, launch children with
  `CREATE_SUSPENDED`, `AssignProcessToJobObject`, then `ResumeThread`
  — this avoids the race where a child spawns grandchildren before
  assignment ([Microsoft Job Objects
  docs](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects),
  [Raymond Chen "Destroying all child
  processes"](https://devblogs.microsoft.com/oldnewthing/20131209-00/?p=2433),
  [Meziantou's
  blog](https://www.meziantou.net/killing-all-child-processes-when-the-parent-exits-job-object.htm)).
  Do not set `JOB_OBJECT_LIMIT_BREAKAWAY_OK` unless intentional.

- **Chrome's experience:** child processes inherit Chrome's job object
  via the sandbox infrastructure, so OS-level cleanup is automatic —
  *but* CEF's `--no-sandbox` path historically left orphans because
  `options.job_handle` was `NULL` ([chromium-dev: Sub process does not
  exit when parent process is
  killed](https://groups.google.com/a/chromium.org/g/chromium-dev/c/ZO43Sra0o7w)).
  For a three-process design (launcher + host + srv), the launcher
  should hold the job, and both the host and the srv should be
  assigned to it. CEF render-process children inherit naturally if
  the host is in the job and breakaway is disabled.

- **Renderer reaping / zombie detection:** Chromium's
  `RenderProcessHostObserver::RenderProcessExited` notifies *after*
  the handle is invalidated (Windows resets the handle, Linux reaps
  the zombie before notification). The fix Chromium chose for
  `BrowserChildProcessObserver` is to *duplicate the handle before
  notifying* so observers can still introspect ([chromium-dev:
  Keeping process handle valid for
  RenderProcessExited()](https://groups.google.com/a/chromium.org/g/chromium-dev/c/X3L10Lyjge8)).
  Lesson: any "process is gone" event must carry whatever metadata
  the consumer needs, because the live handle is already gone by then.

- **Single-instance enforcement:** named mutex (`CreateMutex`) is the
  recommended primitive — kernel-released on process death, no
  stale-file problem ([metric panda games
  guide](https://www.metricpanda.com/rival-fortress-update-46-single-instance-games-on-windows-with-c-cpp/)).
  Lock files require PID validation and crash recovery and are
  universally considered inferior. Electron's
  `app.requestSingleInstanceLock` actually uses Chromium's
  `ProcessSingleton`, which on Windows combines a named mutex, a
  hidden message-window, and a named pipe to hand off command lines
  ([Electron #20268](https://github.com/electron/electron/issues/20268),
  [process_singleton_win.cc behavior in
  #35680](https://github.com/electron/electron/issues/35680)). Known
  failure modes: cross-user-session crashes, headless-no-window edge
  case where second-instance never fires.

  > Caveat for AgentMux: per `CLAUDE.md`, the project explicitly
  > supports running multiple instances in parallel (different
  > versions, dev + portable, multiple portable copies). A
  > **global** named mutex would break that. The right adaptation is
  > a **per-data-dir** mutex name (e.g., `Local\AgentMux-{data-dir-hash}`)
  > so each isolated instance gets its own lock.

- **"Last window closed → exit?":** VS Code and Electron drive this
  *deterministically* through `window-all-closed` (Electron) and
  `LifecycleMainPhase` transitions (VS Code). The Electron canonical
  pattern: subscribe to `window-all-closed` and decide based on (a)
  platform, (b) tray/background mode flag, (c) presence of a
  quit-in-progress flag set by `before-quit`. The "leak" symptom
  ("taskbar entry but no window") is the classic indicator of: pool
  windows counted in the all-windows list, so `window-all-closed`
  never fires; or background mode is implicitly on ([Electron app
  docs](https://www.electronjs.org/docs/latest/api/app),
  [electron-react-boilerplate
  #1863](https://github.com/electron-react-boilerplate/electron-react-boilerplate/issues/1863),
  [Edge's `BackgroundModeEnabled`
  policy](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-browser-policies/startupboostenabled)).

---

## 5. Window pool / pre-warming patterns

The hard-earned invariants from Chromium's spare renderer and Edge's
Startup Boost:

- **Pool items are not in the canonical user-visible list.**
  Chromium's spare renderer is held by
  `SpareRenderProcessHostManager`, separate from the regular
  `RenderProcessHost` registry; it is *adopted* into the regular
  registry only when promoted ([crrev
  2929113002](https://codereview.chromium.org/2929113002)).

- **Pool item never has a taskbar button.** On Windows, the taskbar
  examines `WS_VISIBLE` first, then `WS_EX_APPWINDOW` /
  `WS_EX_TOOLWINDOW` / ownership ([dreamlayers blog on
  `WS_EX_TOOLWINDOW`](https://dreamlayers.blogspot.com/2010/12/hiding-window-from-taskbar-using.html)).
  A pool window must be created from the start with `WS_EX_TOOLWINDOW`
  and without `WS_VISIBLE`, or be a child of a hidden owner — never
  created visible-then-hidden, because toggling style after creation
  requires the hide → `SetWindowLongPtr` → show dance and is fragile.
  CEF supports this via either `windowless_rendering_enabled = true`
  (true OSR) or a `WS_EX_TOOLWINDOW` host frame ([CEF forum: how to
  make cefclient window
  invisible](https://magpcss.org/ceforum/viewtopic.php?f=6&t=15426),
  [chromiumembedded/cef
  #3869](https://github.com/chromiumembedded/cef/issues/3869)).

- **Pool item does not count toward exit decisions.** Chromium's spare
  is destroyed if it doesn't match the requested `BrowserContext` /
  `StoragePartition` and is bounded (Android uses a timeout
  `kAndroidWarmUpSpareRendererWithTimeout`; capped at
  `GetMaxRendererProcessCount`).

- **Promotion is a single atomic transition** that adopts the warm
  item into the canonical registry and re-parents/re-styles the OS
  window in one observable step. Edge's Startup Boost is the analog
  at app level: hidden background processes are explicitly designed
  to *not* count as "the app is running" for UI purposes ([Edge
  Startup
  Boost](https://www.microsoft.com/en-us/edge/features/startup-boost)).

---

## 6. Anti-patterns to avoid

1. **Fire-and-forget IPC for state mutations.** Sender assumes
   success; receiver silently failing produces unbounded drift and
   "lost errors you only hear about from users" ([Service Broker
   fire-and-forget
   anti-pattern](https://davewentzel.com/content/service-broker-demystified-fire-and-forget-anti-pattern/)).
   Use request/response with ack, or event-sourced commit-then-emit.

2. **Locks held across awaits / IPC boundaries.** Classic Rust
   footgun in Tauri: holding a `MutexGuard` across `await` causes
   deadlocks across windows ([Tauri state management
   caveats](https://v2.tauri.app/develop/state-management/)).

3. **Ad-hoc parallel maps that drift.** Chromium's tab strip
   explicitly guards against this with `TabStripModel` as the *only*
   tab registry; "deeper (false and dangerous) assumption that every
   `WebContents` is a browser tab" is called out in the Chromium tab
   helpers doc.

4. **"Soft-closed" windows that aren't really gone.** Reusing a
   `WebContents` / HWND as both "closed" and "hidden in pool" without
   a state distinction is exactly the bug class AgentMux's launcher
   is hitting. Chromium maintains close-animation indices separately
   from the model index for this reason.

5. **Mixed ownership.** Two services both believing they own a
   window's lifetime (e.g., host process and srv both trying to
   release a window on shutdown) reliably produces the "taskbar
   entry, no window" symptom — one process exited but the other still
   has an HWND.

6. **No global ordering across IPC channels.** Mojo style guidance is
   explicit: separate pipes have no FIFO between them, so a single
   subscriber stream per consumer is required for state-sync
   correctness ([Chromium Mojo style
   guide](https://chromium.googlesource.com/chromium/src/+/main/docs/security/mojo.md)).

---

## 7. Recommended skeleton (seed for the AgentMux spec)

**Truth lives in the launcher.** The launcher is the only process
that survives across host crashes and is the only one that can hold
the Job Object. It owns: `Map<WindowId, WindowState>`,
`Map<ProcessId, ProcessRecord>`, the warm pool (separate type), and
the `LifecyclePhase`. The host process is *just an executor* — it
owns HWND handles and reports their state, but doesn't decide.

**Reducer / state machine sits in the launcher** as a Rust
`update(state, command) -> (state, Vec<Event>)` function — pure,
total, exhaustively matched on `WindowState` enum. Pool items live in
a sibling `WarmPool` struct, not in the main map; promotion is a
single command (`PromoteWarm { warm_id, become_window_id }`) that
emits one `WindowAdded` event.

**Process model.** Launcher creates a Job Object with
`KILL_ON_JOB_CLOSE`, no breakaway. Host and srv are launched
`CREATE_SUSPENDED`, assigned, resumed. Single-instance enforced via
**per-data-dir** named mutex (`Local\AgentMux-{data-dir-hash}`),
matching the existing port-file scheme's scope; secondary launches
hand off via named pipe to launcher's reducer as a `BringToFront`
command.

**Frontend mirror.** Each React renderer subscribes to one ordered
event stream (per-renderer Mojo-style channel, or Tauri `emit_to`).
On connect: `GetSnapshot() -> (version, state)`, then apply deltas
where `event.version > local.version`; gap → resnap. All frontend
mutations are typed commands; frontend never assumes success, only
renders on event echo.

**Shutdown.** Reducer-driven phases: `WindowAllClosed` is computed
*excluding the warm pool*; transition to `Quitting` cancels pool
warmup, drains commands, closes host (which triggers Job-object
cascade if anything escapes), then exits launcher. No process can
quit unilaterally — every exit path is a state transition.

---

## Sources

### Chromium
- [Chromium tab_helpers.md](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/tab_helpers.md)
- [Chromium chrome_browser_design_principles.md](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/chrome_browser_design_principles.md)
- [TabStripModel header (legacy)](https://github.com/adobe/chromium/blob/master/chrome/browser/tabs/tab_strip_model.h)
- [SpareRenderProcessHostManager / RenderProcessHostImpl](https://github.com/chromium/chromium/blob/main/content/browser/renderer_host/render_process_host_impl.cc)
- [crrev 2929113002 — Enable spare RenderProcessHost preinitialization](https://codereview.chromium.org/2929113002)
- [Chromium Mojo IDL versioning](https://chromium.googlesource.com/chromium/src/+/master/mojo/public/tools/bindings/README.md)
- [Chromium Mojo style guide / security guidance](https://chromium.googlesource.com/chromium/src/+/main/docs/security/mojo.md)

### Electron
- [Electron BrowserWindow API](https://www.electronjs.org/docs/latest/api/browser-window)
- [Electron Process Model](https://www.electronjs.org/docs/latest/tutorial/process-model)
- [Electron app API (window-all-closed, before-quit, requestSingleInstanceLock)](https://www.electronjs.org/docs/latest/api/app)
- [Electron #20268 — requestSingleInstanceLock without a window](https://github.com/electron/electron/issues/20268)
- [Electron #35680 — single instance lock on Windows](https://github.com/electron/electron/issues/35680)

### VS Code
- [VS Code window management — DeepWiki](https://deepwiki.com/microsoft/vscode/4.2-window-management-and-titlebar)
- [VS Code lifecycle service (browser side)](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/lifecycle/browser/lifecycleService.ts)
- [VS Code #248693 — restart API discussion](https://github.com/microsoft/vscode/issues/248693)

### Tauri
- [Tauri state management](https://v2.tauri.app/develop/state-management/)
- [Tauri Window and Webview Management — DeepWiki](https://deepwiki.com/tauri-apps/tauri/2.3-window-management)
- [Loosely synchronize Zustand stores in multiple Tauri processes](https://www.gethopp.app/blog/tauri-window-state-sync)

### Windows process management
- [Microsoft Job Objects docs](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [Raymond Chen — Destroying all child processes when the parent exits](https://devblogs.microsoft.com/oldnewthing/20131209-00/?p=2433)
- [Meziantou — killing child processes via Job Object](https://www.meziantou.net/killing-all-child-processes-when-the-parent-exits-job-object.htm)
- [chromium-dev — Keeping process handle valid for RenderProcessExited()](https://groups.google.com/a/chromium.org/g/chromium-dev/c/X3L10Lyjge8)
- [chromium-dev — Sub process does not exit when parent process is killed (CEF/--no-sandbox)](https://groups.google.com/a/chromium.org/g/chromium-dev/c/ZO43Sra0o7w)
- [Edge Startup Boost feature page](https://www.microsoft.com/en-us/edge/features/startup-boost)
- [Microsoft StartupBoostEnabled / BackgroundModeEnabled policy](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-browser-policies/startupboostenabled)
- [dreamlayers — hiding a window with WS_EX_TOOLWINDOW](https://dreamlayers.blogspot.com/2010/12/hiding-window-from-taskbar-using.html)
- [CEF #3869 — frameless window/taskbar interaction](https://github.com/chromiumembedded/cef/issues/3869)
- [CEF Forum — making cefclient window invisible](https://magpcss.org/ceforum/viewtopic.php?f=6&t=15426)
- [Metric Panda — single instance games on Windows with C/C++](https://www.metricpanda.com/rival-fortress-update-46-single-instance-games-on-windows-with-c-cpp/)

### State sync / CQRS
- [Service Broker — fire-and-forget anti-pattern](https://davewentzel.com/content/service-broker-demystified-fire-and-forget-anti-pattern/)
- [Salesforce — remote process invocation, fire and forget](https://developer.salesforce.com/docs/atlas.en-us.integration_patterns_and_practices.meta/integration_patterns_and_practices/integ_pat_remote_process_invocation_fire_forget.htm)
- [SAFE-ConfPlanner — CQRS/ES + Elm Architecture sample](https://github.com/SAFE-Stack/SAFE-ConfPlanner)
- [Intents and Facts — pondering CQRS in Elm](https://discourse.elm-lang.org/t/intents-and-facts-pondering-cqrs-in-elm/923)
- [cqrs-es Rust crate](https://crates.io/crates/cqrs-es)
- [esrs — primait/event_sourcing.rs](https://github.com/primait/event_sourcing.rs)
