# Window Reality Reconciliation (WRR) — design

**Status:** Design draft. Awaiting answers to open questions before B.9.1 implementation.
**Author:** AgentA.
**Date:** 2026-04-28 (post-#599 / B.6.1 saga).

---

## TL;DR

Make Win32 HWND state a first-class signal in the launcher's reducer, fed by Win32's own event-publishing mechanisms (`SetWinEventHook` + `WM_WINDOWPOSCHANGED` / `WM_DISPLAYCHANGE`). No polling, no heartbeats. Every drift between CEF browser identity (already tracked) and Win32 reality (newly tracked) is classified at the moment the offending state transition fires and emitted as a typed `Event::HwndDriftDetected`. Phase B.9 ships observation only; Phase F adds self-healing.

---

## Why this exists

### The bug that motivated it

During the B.6.1 smoke-test session on 2026-04-28, manual `curl` against the live host's IPC server invoked `open_new_window`, which created a `FullInstance` CEF browser and Win32 taskbar entry. The window was not visible to the user (off-screen, behind the main window, or otherwise non-foregrounded). When the user later closed the main window, the curl-created secondary remained alive — a stray taskbar entry attached to no user-visible window.

### Why the current state machine missed it

The reducer in its post-B.5 form tracks **identity** but not **observability**:

- `state.windows` (launcher mirror) tracks `{label, kind, parent_label}`.
- `state.browsers` (host scaffolding) tracks the live CEF `Browser` handles by label.
- `report_window_opened` / `report_window_closed` keep both sides in lockstep.

The drift sensor (B.4b) compares **counts**: `host.browsers.len() == launcher.windows.len()`. Both sides can be **identically wrong about Win32 reality** — the curl-created window IS in `state.browsers` and IS in `state.windows`, so no drift fires. The user just can't see it.

The structural gap: the reducer never observes Win32 HWNDs. Visibility, foreground status, position, monitor membership are all invisible to it. So is the existence of HWNDs that the host didn't create through its own `OnAfterCreated` path (none currently, but a defense-in-depth sensor would catch a bug class we haven't enumerated).

---

## Design goal

Treat Win32 HWND state as a **first-class projection** in the launcher reducer. The reducer learns:
- Which HWNDs exist in the host process.
- Which CEF browser label each HWND belongs to.
- Each HWND's visibility, iconic-state, last-foreground time, last-known rect.
- The current monitor topology.

Every change to any of those is dispatched through the existing IPC pipe as a typed `Command`. Drift between CEF identity (the existing `state.windows`) and Win32 reality is classified per-transition in the reducer and emitted as `Event::HwndDriftDetected`. Phase B keeps it observation-only; Phase F gates corrective action behind config.

**Non-goal:** polling. The reducer never wakes up to scan state. Every emission is in response to a specific OS event the reducer received as a Command.

---

## Event sources (zero polling)

Every transition we care about already fires a Win32 event we can hook synchronously. One `SetWinEventHook(WINEVENT_OUTOFCONTEXT)` install at host startup, one teardown at shutdown. Each callback is a pre-existing OS notification — no thread-pool worker waking on an interval, no timer.

| Transition | OS event | Subscription |
|---|---|---|
| HWND created | `EVENT_OBJECT_CREATE` (objid=`OBJID_WINDOW`) | `SetWinEventHook(WINEVENT_OUTOFCONTEXT)` — process-local, no DLL injection |
| HWND destroyed | `EVENT_OBJECT_DESTROY` | same hook |
| Visibility changed | `EVENT_OBJECT_SHOW` / `EVENT_OBJECT_HIDE` | same hook |
| Foreground / focus | `EVENT_SYSTEM_FOREGROUND` | same hook |
| Min / restore | `EVENT_SYSTEM_MINIMIZESTART` / `EVENT_SYSTEM_MINIMIZEEND` | same hook |
| Move / resize | `WM_WINDOWPOSCHANGED` | wrap host's existing wndproc |
| Monitor topology change | `WM_DISPLAYCHANGE` | wrap host's existing wndproc |
| CEF browser create / close | `OnAfterCreated` / `OnBeforeClose` | already hooked (B.4) |

`SetWinEventHook` with `WINEVENT_OUTOFCONTEXT` runs the callback on the calling thread of the hooking process — no DLL injection into other processes, no extra threads. We filter to the host process's PID via the `idProcess` parameter.

---

## State machine extension

### New per-window state in the launcher

```rust
struct WindowState {
    // Existing — identity
    label: String,
    kind: WindowKind,
    parent_label: Option<String>,
    // NEW — observability axis
    hwnd: Option<u64>,                     // populated by HwndOpened
    visible: bool,                         // last-known visibility
    iconic: bool,                          // minimized?
    last_rect: Option<Rect>,               // last-known position/size
    last_foreground_at_ms: Option<u64>,    // ms since launcher start
}
```

### New global state

```rust
struct State {
    // ... existing fields ...
    // NEW
    monitors: Vec<Rect>,                   // updated on TopologyChanged
    /// HWNDs the reducer has seen but couldn't yet associate with a
    /// label (transient: race between OS create event and CEF
    /// `OnAfterCreated` reaching the reducer). Drained on each
    /// reconciliation.
    pending_hwnds: HashMap<u64, HwndPending>,
}
```

### New IPC commands

One per OS event, no aggregation. Wire types live in `agentmux-common::ipc`:

```rust
Command::ReportHwndOpened {
    hwnd: u64,
    class_name: String,
    title: String,                         // GetWindowTextW snapshot at create
    label_hint: Option<String>,            // host's best guess from
                                           // pending_window_creations,
                                           // None if it can't tell yet
}
Command::ReportHwndDestroyed { hwnd: u64 }
Command::ReportHwndVisibilityChanged { hwnd: u64, visible: bool }
Command::ReportHwndForegroundChanged { hwnd: u64 }    // "this hwnd is now FG"
Command::ReportHwndIconicChanged { hwnd: u64, iconic: bool }
Command::ReportHwndPositionChanged { hwnd: u64, rect: Rect }
Command::ReportMonitorTopologyChanged { rects: Vec<Rect> }
```

### New IPC events

```rust
Event::HwndDriftDetected {
    kind: HwndDriftKind,
    label: Option<String>,
    hwnd: Option<u64>,
    detail: String,
    severity: Severity,
    version: u64,
}

enum HwndDriftKind {
    /// state.windows has a label whose `hwnd` field never got
    /// populated, AND a follow-up event arrived that should have
    /// reconciled it. CEF says open, Win32 has no matching HWND.
    BrowserWithoutHwnd,
    /// HWND in the snapshot doesn't map to any state.windows /
    /// state.pool label. Win32 has it, CEF didn't open it. The
    /// "stray taskbar" case.
    HwndWithoutBrowser,
    /// HWND for a known label was never SHOWN since open and another
    /// event has elapsed. User can't see it.
    HiddenSinceOpen,
    /// Window rect doesn't intersect any monitor in state.monitors.
    /// Off-screen orphan.
    OffMonitor,
    /// HWND destroy event arrived without a preceding
    /// ReportWindowClosed for the matching label. Renderer crashed,
    /// took the HWND with it.
    OrphanDestroy,
    /// ReportWindowClosed arrived, but subsequent OS events for the
    /// HWND keep firing (it never went away on the Win32 side).
    LingeringHwnd,
}

enum Severity { Info, Warn, Error }
```

---

## Drift classification — at event time, no tick

Each emission below is **emitted only when a specific OS-driven Command is dispatched through the reducer**. The reducer never wakes up to scan state. All comparisons happen synchronously inside `update()` for the command that just arrived.

| Scenario | Triggering Command | Reducer action |
|---|---|---|
| **Stray HWND** | `ReportHwndOpened { hwnd, class_name, label_hint }` where `label_hint` is None and there is no pending `state.windows` entry awaiting an HWND | Emit `HwndDriftDetected { kind: HwndWithoutBrowser, hwnd }` |
| **Browser-without-HWND** | Any later event whose dispatch finds a `state.windows[label]` with `hwnd == None` long after `ReportWindowOpened` arrived (the host should have followed up with `ReportHwndOpened` in the same dispatch cycle) | Emit `BrowserWithoutHwnd` once (de-duplicated by label) |
| **Off-monitor** | `ReportHwndPositionChanged { hwnd, rect }` where `rect` does not intersect any rect in `state.monitors`, OR `ReportMonitorTopologyChanged` that strands an existing window's last rect | Emit `OffMonitor` synchronously in the same `update()` |
| **Hidden after open** | `ReportHwndVisibilityChanged { hwnd, visible: false }` AND no `ReportHwndForegroundChanged` had arrived for that hwnd between `ReportHwndOpened` and now | Emit `HiddenSinceOpen` |
| **Destroy without close** | `ReportHwndDestroyed { hwnd }` where the matching label in `state.windows` has `hwnd == this hwnd` AND no preceding `ReportWindowClosed { label }` was processed | Emit `OrphanDestroy`; clean the entry |
| **Close without destroy** | Any subsequent OS event for an HWND that no longer has a matching `state.windows` label (its label was already removed via `ReportWindowClosed`) | Emit `LingeringHwnd` |

### What pure event-driven gives up

Pure event-driven catches **transitions**, not steady-state oddities. Specifically:

- "Window has been minimized for 60 seconds" cannot fire spontaneously — there is no clock event to trigger evaluation.
- The reducer **can** opportunistically evaluate at the next event from any source, using the timestamp it already gets in `Ctx::now_rfc3339`. If the user takes any action (clicks anything, opens any window, moves any window), the reducer re-evaluates all `state.windows` entries' staleness as a side effect of dispatching THAT command. That is not a heartbeat — it is free riding on whatever events naturally occur.
- Truly idle, untouched windows don't get spontaneous drift events. That is correct: if nothing is happening and nothing is changing, nothing is wrong by definition.

---

## Module layout

```
agentmux-cef/src/wrr/
    mod.rs          # public API: install_hooks(), uninstall_hooks()
    win_event.rs    # SetWinEventHook callback; routes to launcher_ipc
    wndproc.rs      # WM_WINDOWPOSCHANGED / WM_DISPLAYCHANGE hooks (wraps host's existing wndproc)
    classify.rs     # is_app_class(class_name), rect-vs-monitor-set helpers

agentmux-launcher/src/wrr/
    mod.rs          # reducer arm + drift classification
    rect.rs         # rect intersection / monitor membership math

agentmux-common/src/ipc.rs
    + Command::ReportHwnd* (7 variants)
    + Command::ReportMonitorTopologyChanged
    + Event::HwndDriftDetected
    + HwndDriftKind, Severity, Rect
```

No sensor.rs. No 1-Hz loop. The WRR subsystem is (a) a thin Win32 hook layer in the host that turns OS events into IPC Commands, and (b) a reducer arm in the launcher that classifies state transitions.

---

## Phase mapping

| Phase | Scope | Approx LoC | Behavior |
|---|---|---|---|
| **B.9.1** | `wrr/win_event.rs` + `wndproc.rs` hook installation in host; new IPC commands wired; reducer state additions + reducer arm + drift classification + emission | ~400 | OBSERVATION ONLY. Drift events logged at the configured severity floor. No corrective action. |
| **B.9.2** | `agentmux.exe --diag wrr` Tool client. Subscribes as `ClientKind::Tool`, prints last N drift events with timestamps and current `state.windows` extended view. | ~80 | Operator visibility. "Did anything orphan in this session? Show me the table." |
| **F.WRR** | `wrr/enforcer.rs` in host. Subscribes to `Event::HwndDriftDetected`. Gated on `enforce=true` config. Corrective UI tasks: auto-raise off-monitor windows, close stray HWNDs, surface "we lost a window" notification to user. | ~250 | SELF-HEALING. |

### Why land B.9.1 first as observation only

Without enforcement, B.9.1 is pure addition: no risk of self-healing logic doing the wrong thing. The drift events get logged; we run a session, smoke, and read the launcher log to see what bugs were lurking that we hadn't noticed. That data informs B.9.2's `--diag` output and Phase F's enforcement policy (which corrections are safe, which need user confirmation, which thresholds make sense).

---

## Configuration

Operator-tunable, defined in `WrrConfig` in both crates and overridable via env vars (so portable users can tweak without rebuilding):

| Knob | Default | Env var | Effect |
|---|---|---|---|
| `enabled` | `true` | `AGENTMUX_WRR` | Master switch. `false` skips hook installation entirely. |
| `severity_floor` | `Warn` | `AGENTMUX_WRR_SEVERITY` | `Info` / `Warn` / `Error`. Events below the floor are not broadcast (still logged at DEBUG). |
| `enforce` | `false` | `AGENTMUX_WRR_ENFORCE` | Phase F only. Default false through end of Phase B. |
| `app_class_allowlist` | `["CefBrowserWindow", "Chrome_WidgetWin_*"]` | `AGENTMUX_WRR_APP_CLASSES` | Window classes the host hook considers candidate AgentMux windows. Filters out tooltips, IME hooks, CEF subprocess HWNDs, etc., at the hook callback (don't waste an IPC dispatch on noise). |
| `dedup_window_ms` | `2000` | `AGENTMUX_WRR_DEDUP_MS` | Don't re-emit the same `HwndDriftKind` for the same `(label, hwnd)` pair within this window. Prevents log flooding when a transition oscillates. |

---

## Open questions

1. **Naming.** Is `wrr` / "Window Reality Reconciliation" the right module name? Alternatives considered: `window_reality`, `hwnd_recon`, `hwnd_state`. Pick one before committing — renaming later is churn.
2. **Sensor location.** All hooks live in the host (closest to CEF, no IPC for the OS events themselves — just for the resulting Commands). Alternative: separate small "wrr-sensor" process attached to the host's PID via `SetWinEventHook` with `idProcess`. Trade-off: the dedicated process survives a host crash long enough to report the destroy event, which could catch one extra failure mode (host crashes mid-window-create, no `ReportWindowClosed`, destroy event still flushes). Per the conversation, host-internal is the cheaper start.
3. **CEF subprocess HWNDs filter.** Filter at the hook level (skip class names matching `Chrome_RenderWidget*`) or pass through and let the reducer ignore? Strong lean: filter at hook to keep the wire light and keep the reducer's `state.windows` semantics clean (only "user-meaningful" HWNDs).
4. **Drift severity floor for B.9.1.** All-WARN, or split (`OrphanDestroy` / `HwndWithoutBrowser` = ERROR, `OffMonitor` / `HiddenSinceOpen` / `LingeringHwnd` = WARN, `BrowserWithoutHwnd` = INFO since it's transient by nature)?
5. **Backpressure.** OS events can fire bursts (e.g. monitor topology change → all windows resize → many `WM_WINDOWPOSCHANGED`). Should the host coalesce on the way out (drop redundant `ReportHwndPositionChanged` for the same hwnd within a few ms) or send everything and let the reducer dedupe via `dedup_window_ms`? Lean: coalesce in the host, since the IPC pipe is the cheap-but-not-free bottleneck.

---

## Where this fits in the broader Phase mapping

This addition does NOT change Phase B's existing sub-PR sequence. B.7.3 (CEF JS bridge for typed launcher events) and B.8 (Phase B exit: property tests, `--diag` tool, CI smoke) remain. WRR is **B.9** — added because the conversation surfaced a class of bug B.7/B.8 wouldn't catch.

The Phase F multi-reducer destination is a natural fit for WRR's enforcement role: the host's `wrr/enforcer.rs` becomes part of the host-side reducer's saga set. Until Phase F, B.9.1 + B.9.2 deliver visibility; corrections stay manual.

---

## How to start B.9.1

Concrete first PR plan, in commit order:

1. `agentmux-common/src/ipc.rs` — add the new `Command` / `Event` variants + `HwndDriftKind` / `Severity` / `Rect`. No behavior change.
2. `agentmux-cef/src/wrr/{mod.rs, win_event.rs, classify.rs}` — `SetWinEventHook` install / teardown, callback that turns OS events into IPC Commands, class-name filter.
3. `agentmux-cef/src/wrr/wndproc.rs` — wrap the existing host window proc to capture `WM_WINDOWPOSCHANGED` and `WM_DISPLAYCHANGE`, dispatch as `ReportHwndPositionChanged` / `ReportMonitorTopologyChanged`.
4. `agentmux-launcher/src/wrr/{mod.rs, rect.rs}` + reducer arm in `update()` — drift classification, emission. State additions (`hwnd`, `visible`, `iconic`, `last_rect`, `last_foreground_at_ms` per `WindowState`; `monitors` and `pending_hwnds` global).
5. `WrrConfig` env-var parsing on both sides. Defaults gate B.9.1 to observation-only.
6. **Smoke test before merge** (per `feedback_verify_before_push`):
   - Open AgentMux portable.
   - `curl` `open_new_window` (the bug we just had).
   - Verify `Event::HwndDriftDetected { kind: HiddenSinceOpen | OffMonitor }` lands in the launcher log within one OS event tick.
   - Move a window off all monitors → verify `OffMonitor` fires.
   - Force-kill the host's main browser (renderer crash via Task Manager) → verify `OrphanDestroy` fires.

Once 1–6 land, B.9.1 is done and we have visibility. B.9.2 is a small `--diag` follow-up. Phase F adds enforcement.
