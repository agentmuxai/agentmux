# Browser pane input owner: one state machine per window

**Status:** active. Phase 1, the pure machine (`crates/cef/src/input_owner/`), built in #4970. Phases 2 to 4 (the
Windows adapter, the frontend contract, the macOS and Linux adapters) not yet built.
**Date:** 2026-10-15.
**Requested by:** repo owner (asafebgi), after a browser pane stopped taking the wheel and typing until a tab switch:
"we need really rigorous state machine for the browser's input focus … take the lead, design the machine and implement".
**Author:** AgentX@narko.
**Evidence and current code:** `docs/reports/REPORT_BROWSER_PANE_INPUT_FOCUS_STATE_2026_10_15.md` (gaps G1–G11 below are
that report's §3.2).
**Supersedes, once Phase 2 is built:** `SPEC_BROWSER_PANE_FOCUS_LOCK.md` (never built), and the focus paths of
`SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md`.

## 1. The problem in one paragraph

A window's keyboard goes to the app's own content or to one browser pane's page. Today about a dozen places decide which
(host Rust, frontend TypeScript and Chromium), through one process-wide "allow the next focus" flag, with no record of who
should have the keyboard and no check that Windows agrees. Focus messages are sometimes swallowed, so a pane can hold the
Win32 keyboard focus while Chromium thinks it is blurred and keys go nowhere. Mouse input to a pane depends on its window
being on top, which nothing maintains. Each symptom recovers when something unrelated resets it.

## 2. Principles

1. **One owner of input per window,** in the host, because only the host can see and change Win32 focus, activation and
   window order. The frontend owns the **selection** (what the user means); the host owns **input** (which surface gets
   the keys) and reports it back.
2. **A pure core.** `input_owner::machine` has no Win32, CEF, locks, clock or logging. Events carry the time.
   `step(&mut WindowInput, Event, now_ms) -> Vec<Effect>`. Same shape as the host reducer (`crates/cef/src/reducer`).
3. **Observe, don't fight.** Focus messages always reach Chromium (never swallowed). Corrections are posted afterwards,
   on the UI thread, never from inside another window's handler.
4. **Enforce only what was asked for.** The host sees focus arrive at a pane's windows, not at the app's own content: a
   click on the app moves Win32 focus a few milliseconds before the frontend's selection arrives. So the machine
   corrects a focus change only while it has asked for an owner (`Pending`) or when a page takes the keyboard on its own.
   A drift toward the app is recorded, not reversed, and taken as the user's new selection.
5. **The user's last action wins; a request never fires late.** A selection that can't be applied yet (its pane isn't
   live) is kept for at most 1.5 s, the same expiry as macOS's parked pane-key request (#4958), and is never applied
   by a later layout pass after that. #4958 fixed exactly this on macOS: an old request applied on a layout change took
   the keyboard mid-typing.
6. **Verify by asking Windows.** After every request the machine schedules a check of `GetFocus()`; a mismatch retries a
   bounded number of times, then is accepted and logged as a warning.
7. **Pointer input is part of the state.** Live panes are raised above the app's own child windows when the window is
   shown or activated, so a visible pane always takes its wheel and clicks.
8. **Every transition is logged** in one line, `[input]`, by the adapter.

## 3. The machine

### 3.1 State, per top-level window

```text
WindowInput {
  active: bool                 // the window is the foreground window
  want:   Owner, want_at_ms    // the user's selection (App | Pane(block)) and when it was made
  have:   Owned(Owner) | Pending { target: Owner, attempt } | Unknown
  panes:  block -> Live | Collapsed | Closing
  last_restore_ms, restore_due // the rate limit on correcting a page's own focus()
}
Surface = App | Pane(block) | Elsewhere      // where Win32 focus was seen
```

`Pending` replaces the global `ALLOW_BROWSER_PANE_FOCUS_ONCE`: it belongs to one window and names one target.

### 3.2 Events and effects

| Event | Source (Windows adapter) |
|---|---|
| `Select(owner)` | `browser_pane_focus` → `Pane`, `main_window_focus` → `App` (Phase 3: one `input_select` IPC) |
| `PanePointerDown(b)` | `WM_LBUTTONDOWN` in b's windows |
| `AppPointerDown` | macOS: a click in the app window (`swizzled_nsapp_send_event`); Windows: not observed, the frontend's `Select(App)` stands in |
| `FocusGained(surface)` | `WM_SETFOCUS` in a pane's windows (after Chromium has handled it) |
| `Activated` / `Deactivated` | top-level `WM_ACTIVATE`, posted to run after CEF's own handling |
| `WindowShown` | top-level `WM_SHOWWINDOW` / restore |
| `PaneLive` / `PaneShown` / `PaneCollapsed` / `PaneClosing` / `PaneGone` | pane create, rect apply (0×0 is collapsed), close |
| `Verify { seq, surface }` | the posted check: `GetFocus()` mapped to a surface. Only the latest check asked for counts; an older `seq` belongs to an earlier request and is ignored |

| Effect | What the adapter does, on the UI thread |
|---|---|
| `FocusApp` | `set_focus(1)` on the window's browser, Win32 `SetFocus` on its render widget, `set_focus(0)` on its panes |
| `FocusPane(b)` | `set_focus(1)` on b, Win32 `SetFocus` on b's window |
| `RaisePanes` | `SetWindowPos(HWND_TOP, no move/size/activate)` on every live, shown pane of the window |
| `Verify { seq, after_ms }` | post a delayed check that sends `Verify { seq, surface }` |
| `Warn(reason)` | log a warning with the window's state |

### 3.3 Transitions

Write `focus(o)` for `FocusApp` or `FocusPane(b)`, and "usable" for `App`, or a pane that is `Live` (not collapsed or
closing). A selection that isn't usable falls back to `App` when it is applied.

| Event | Condition | New state | Effects |
|---|---|---|---|
| `Select(o)` | — | `want = o` | then as below |
| | window not active | unchanged | none: applied on `Activated` |
| | `o` already owned | unchanged | `Verify(0)` (cheap; catches a stale record) |
| | otherwise | `Pending(usable(o))` | `focus`, `Verify(50)` |
| `PanePointerDown(b)` | — | as `Select(Pane b)` | |
| `AppPointerDown` | — | as `Select(App)` | |
| `FocusGained(s)` | `s` is the pending target | `Owned(s)` | none |
| | `s` is the current owner | unchanged | none |
| | `s` is `Elsewhere`, or the window is inactive | `Owned(s)` | none |
| | a page took it, restore allowed | `Pending(usable(want))` | `focus`, `Verify(50)` |
| | a page took it within 100 ms of the last restore | `Owned(s)`, `restore_due` | `Verify(remaining)` |
| `Activated` | — | `active`, `Pending(usable(want))` | `focus`, `RaisePanes`, `Verify(50)` |
| `Deactivated` | — | `!active` | none |
| `WindowShown` | — | unchanged | `RaisePanes` |
| `PaneLive(b)` / `PaneShown(b)` | b is `want`, chosen within 1.5 s, window active, not owned | `Pending(b)` | `FocusPane(b)`, `Verify(50)` |
| `PaneCollapsed(b)` | b owns or is pending | `Pending(App)` | `FocusApp`, `Verify(50)`. Showing b again doesn't take the keys back by itself: the frontend's selection, now that the app has them, decides (`focusManager.ts`) |
| `PaneClosing(b)` / `PaneGone(b)` | b owns or is pending | `Pending(App)`, `want = App` if it was b | `FocusApp`, `Verify(50)` |
| `Verify(s)` | `seq` isn't the latest asked for | unchanged | none |
| | `Pending(t)` and `s == t` | `Owned(t)` | none |
| | `Pending(t)`, mismatch, attempt < 2 | `Pending(t, attempt + 1)` | `focus(t)`, `Verify(50)` |
| | `Pending(t)`, mismatch, attempt = 2 | `Owned(s)` | `Warn` |
| | `restore_due`, 100 ms since the last correction | `Pending(usable(want))` | `focus`, `Verify(50)` |
| | `restore_due`, sooner | unchanged | `Verify(remaining)` |
| | `Owned(o)`, `s != o` | `Owned(s)`; `want = App` if `s` is the app | none (drift recorded, not reversed) |

### 3.4 Invariants (checked by the tests after every step)

- **I1:** `Pending` and `Owned` never name a pane that is collapsed, closing or unknown, except `Owned` of what Windows
  reports (a recorded fact).
- **I2:** every `Pending` is followed by a `Verify` effect in the same step.
- **I3:** at most one `focus` effect per step.
- **I4:** an inactive window produces no `focus` effects (focusing activates a window: the 2026-10-07 focus storm).
- **I5:** a page's own focus is corrected at most once per 100 ms per window.
- **I6:** every `focus` effect names `App` or the current `want`.
- **I7:** a selection older than 1.5 s is never applied by a pane becoming live or shown.

## 4. Phases

| Phase | Content |
|---|---|
| 1 | This spec; `crates/cef/src/input_owner/` pure machine with transition and property tests. No behaviour change. |
| 2 | Windows adapter: per-window registry, the events above wired in, effects run on the UI thread, `[input]` logging, `GetFocus` verify. Removes `ALLOW_BROWSER_PANE_FOCUS_ONCE`, the swallowed `WM_SETFOCUS`, the pre-CEF `WM_ACTIVATE` restore and the off-thread `SetFocus`. Adds `RaisePanes` and the collapse hand-off. Live-tested in an isolated dev build. |
| 3 | Frontend: one `input_select` with a sequence number for every selection change; an `input-owner-changed` event back; `document.hasFocus()` no longer stands in for "a pane has the keys". |
| 4 | macOS (key window) and Linux (Views focus) adapters onto the same core. |

## 5. Not in scope

- Which DOM element inside the app gets the caret: unchanged. That is `focusManager.ts` (the caret follows the selection,
  #4479; an opened input takes the caret, #4481; the window-focus net that waits 100 ms and yields to a click, #4958)
  and `userCaretInBlock`. The machine decides only app vs. pane.
- Browser panes as Views overlays on Windows (#4430): it would change the window tree, not the need for one owner.
