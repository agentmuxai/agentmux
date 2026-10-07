# Incident: restored windows fight over focus forever

**Date:** 2026-10-07
**Author:** Agent4
**Status:** fixed in the PR that adds this file (`MainFocusReclaimTask` no longer activates a window that isn't already the active one).
**Seen in:** Agent4's own `task dev` window on `main` @ `60bcc3e99` (v0.59.12), Windows. No experimental switches were on.

## 1. What happened

The operator saw "a bunch of windows opened and they were selecting each other over and over". The dev instance was stopped about 43 minutes after it started. By then the host log was 668 MB.

- **07:56:46–47:** at startup the launcher's snapshot had no windows (`windows: []`), so the host took the reproject slow path, recreating windows from srv. srv had **8** restorable windows for this dev data directory, and all 8 opened within one second (`[reproject] recreated window` ×8, `had_rect: false`).
- **07:56:56–57:** each new window registered, and the old srv window id it replaced was closed (`[reproject] new window confirmed live`), as designed.
- **From 07:56:57:** the 8 windows took turns being activated, and never stopped. Over the run:
  - **359,053** `main_window_focus` IPC calls, about 45,000 per window;
  - as many `MainFocusReclaimTask` runs, each a `host.set_focus(1)` plus a Win32 `SetFocus`;
  - as many `WM_ACTIVATE` focus restores.

  The hops ran about 6 ms apart, while each IPC took 45–95 ms to answer.

Earlier runs on the same dev data opened one window and logged 3–62 `main_window_focus` calls in total.

## 2. The loop

1. **A window is activated.** Its `WM_ACTIVATE` observer (`[focus-restore]`) calls `SetFocus` on the window's render widget, so the page regains focus.
2. **The page's focused element gets `focusin`.** `block.tsx`'s `handleChildFocus` calls `getApi().reclaimWindowFocus(windowLabel)`, which is the `main_window_focus` IPC. It's meant to pull Win32 keyboard focus back from a browser-pane HWND to the window's own render widget.
3. **The IPC posts `MainFocusReclaimTask`** for that window. It calls `host.set_focus(1)` and Win32 `SetFocus` on the window's render widget.
4. **If another window is active by then, `SetFocus` activates this one.** Another window's request ran in the 45–95 ms this request was in flight, so another window usually is active. Windows activates the target's top-level window, the previous window deactivates, and this window gets `WM_ACTIVATE`: back to step 1.

**One window can't loop:** step 4 finds it already active, so nothing new is activated and no new `focusin` follows. **With two or more windows,** each window's in-flight reclaim re-activates it after another window's reclaim has taken activation away. Every activation yields one more reclaim, so the windows trade activation forever. Restoring several windows at once starts a loop in every one of them.

The IPC exists to move keyboard focus *inside* a window: from a browser-pane HWND to the page, when the user clicks into the page. It was never meant to change *which* window is active, but nothing in `MainFocusReclaimTask` prevented that.

## 3. Why there were 8 windows

The dev instance on this data directory had been force-stopped many times during a day of performance testing, and never quit gracefully. The reproject slow path already defends against window rows piling up:
- it garbage-collects rows that never registered;
- it closes a replaced old window id only once its replacement confirms it's live;
- it caps recreation at `MAX_SLOW_PATH_RECREATE` (20) per launch.

The 8 that were recreated were real, restorable windows, so every launch on that data would open 8. A user who quits with several windows open restores several windows too. So **the focus loop is the defect to fix.** The pile-up is a test artifact, already bounded by existing mitigations.

## 4. Fix

`MainFocusReclaimTask` (`crates/cef/src/ui_tasks/window.rs`) now acts only when the target window is the foreground window (Windows). It resolves the target's top-level HWND first and compares it with `GetAncestor(GetForegroundWindow(), GA_ROOT)`. If they differ, it does nothing:
- **neither `host.set_focus(1)` nor Win32 `SetFocus`,** since either can activate a background window;
- **nothing when no AgentMux window is foreground,** because then a reclaim would steal focus from another app.

The decision is a pure function, `reclaim_allowed(target_root, foreground_root)`, with unit tests.

**What still works:**
- Clicking into a window's page after a browser pane had keyboard focus: the window is already foreground (the click activated it), so the reclaim runs as before.
- A host-forwarded app shortcut from a browser pane (`keymodel-dispatch.ts`): the key was pressed in that window, so it's foreground.
- The pane-destroy hand-off (empty label): it already resolves the foreground window itself, so it's unchanged.

**What stops:** a background window's page, focused by its own restore or activation, can no longer take activation from the window the user is in. Without the activation hop the loop has nothing to feed it: a reclaim for an inactive window doesn't activate it, so no new `focusin` follows.

### 4.1 Verification, and what it doesn't show

- **Unit tests:** `reclaim_tests` in `ui_tasks/window.rs` cover the decision: reclaim in the foreground window; not in a background window; not when no AgentMux window is foreground; old behaviour when the target is unknown.
- **Not reproduced live with two windows.** Agent4's dev build restored two windows at once in three setups (plain; a focused terminal in each; a terminal and a browser pane in each). Without the fix, these made 2, 2 and 3 `main_window_focus` calls in about 30 s. With it, 2 and 5. So a pair of windows doesn't trigger the loop. It evidently needs more of what the incident had: eight windows confirming live in the same second, each with a restored workspace of agent, terminal and browser panes, and 45–95 ms IPC round trips under load. The incident's own data (8 windows) would reproduce it, but wasn't rerun: the machine was near its commit limit, and the loop is disruptive on screen.
- **What supports the fix:**
  - **The incident log:** every hop in the loop is a reclaim for a window that isn't the active one, immediately followed by that window's `WM_ACTIVATE`.
  - **Win32 behaviour:** `SetFocus` on a window in another, inactive top-level of the foreground thread activates that top-level.
  - **The fix removes exactly that step,** and the two-window runs show it leaves normal focus traffic unchanged.

## 5. Follow-ups

- [ ] Reproduce the loop deterministically, for example with a test harness that restores N windows each with a focused block and injects IPC latency. Then confirm the fix against it.

- [ ] macOS and Linux reclaim through `host.set_focus(1)` only. They weren't seen looping, but whether `set_focus` on a background window activates it there is unchecked.
- [ ] `block.tsx`'s `handleChildFocus` sends a reclaim on every `focusin`, including focus restored by window activation. It could skip when focus is already where it should be, but that's no longer needed to stop the loop.
- [ ] Restoring several windows at once still gives each page a startup focus claim. With the fix, only the foreground one acts.
