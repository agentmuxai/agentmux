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
   live) is kept for at most 1.5 s, the same expiry as macOS's parked pane-key request (#4958). After that it is
   dropped and the app is the selection, so neither a later layout pass nor a later activation applies it. #4958 fixed exactly this on macOS: an old request applied on a layout change took
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
| `FocusGained(surface)` | `WM_SETFOCUS` in a pane's windows (after Chromium has handled it), and CEF's `on_got_focus` for a pane's browser: a new page's render widget window is created after the pane's windows are subclassed, so focus it takes on load never reaches a subclass |
| `Activated` / `Deactivated` | top-level `WM_ACTIVATE` and `WM_ACTIVATEAPP`, effects posted to run after CEF's own handling. Only the foreground window is `Activated`: Windows also activates a window in the background when something in it takes focus while another app is in front (a new pane's page does), and the app's focus reclaim rightly refuses to run in a window the user isn't in. If the window isn't the foreground yet, the adapter looks once more 50 ms later; otherwise `WM_ACTIVATEAPP` reports it when it comes forward. An activation before the window's hook was installed is never reported, so the adapter also checks the foreground when the hook is installed and before applying a selection |
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
| `PaneLive(b)` / `PaneShown(b)` | b is `want`, parked (it couldn't take focus when chosen), chosen within 1.5 s, window active | `Pending(b)` | `FocusPane(b)`, `Verify(50)` |
| `PaneCollapsed(b)` | b owns or is pending | `Pending(App)` | `FocusApp`, `Verify(50)`. If b was the selection, the app is now: neither showing b again nor activating the window takes the keys back without a fresh selection; the frontend's selection, now that the app has them, decides (`focusManager.ts`) |
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
- **I8:** a selected pane can take focus now, or is waiting for it (parked) within 1.5 s of being selected; otherwise
  the app is the selection.

### 3.5 Diagnostics (Windows)

The wheel and typing failures take long use to reproduce, so each one has to be readable from the log alone. Besides
the `[input]` line for every change of state, `input_owner/watch.rs` installs a thread-local `WH_GETMESSAGE` hook on
the UI thread, which sees every message the thread takes off its queue. It samples the wheel, left clicks and key
presses, at most one of each kind per 250 ms, takes no lock, and posts the check to a UI task. A pane counts as under
the pointer only if its top-level window is the one on top there, so a floating window or another app in front hides
it, while a window inside the same top-level covering it is reported. A `[input-watch]` warning is logged, at most one per kind per 5 s with a count of the
rest, when:

- a wheel or click over a visible pane went to another window, or to a pane it wasn't over. The line names the window
  that got it, the window at that point, and the input owner's state;
- keys pressed in a window in front went to no window of it, or to a surface other than the one the input owner
  believes has the keyboard, or the input owner counts that window inactive.

No key codes are read. The per-message wheel and key lines in the pane subclass are debug-level.

### 3.6 The frontend's half

- **One request, numbered.** Every request for the keyboard goes out as `input_select { window_label, block_id?, epoch,
  seq }`: the app's own content without `block_id`, a browser pane with it. The existing `reclaimWindowFocus` and
  `browserPanes.focus` send it (`cef-host-commands.ts`), so every caller is numbered without changing. The requests are
  separate IPCs and can reach the host out of order; the host drops one whose `seq` isn't above the last one it applied
  for that window (`registry::apply_selection`, which applies it under the same lock). `epoch` is when the page loaded (microseconds), so a reloaded page starts its own
  numbering and a request still in flight from an earlier page, even its first, is dropped. A page load more than a
  minute older than the current one is a new page after the clock was set back, not a late request.
  On every platform; `main_window_focus` and `browser_pane_focus` stay for other callers.
- **The host says what happened.** When whether the window is active, or where its machine last saw the keyboard,
  changes, the window's page gets `input-owner-changed { active, keys: "app" | "pane" | "elsewhere" | null, block_id }`
  (`registry::report_if_changed`). While a request is pending the keyboard is reported where it was last seen.
  `input_owner_state` returns the same for a page that has just loaded. Windows only until Phase 4; elsewhere it is null.
- **`userInThisWindow()`** (`input-owner.ts`): the page has focus, or the window is active and one of its panes has the
  keyboard. It replaces `document.hasFocus()` where that read "a pane has the keys" as "the user is elsewhere": the
  caret-follows-selection net (`focusManager.ts`), the native menu's command (`command-registry.ts`) and the window
  focus signal (`window-focus.ts`). Where nothing reports, it is `document.hasFocus()`, as before.

### 3.7 The macOS and Linux adapter (Phase 4)

Neither platform has an input owner today. On macOS the pieces are hooks in `ui_tasks/platform_macos.rs` and one
parked request for the whole app (`PENDING_PANE_KEY`, #4958); on Linux the host has almost no focus logic. The same
bug classes the Windows adapter removed are there: one global slot shared by every window, a reclaim and a key change
made in a window the user isn't in (`MainFocusReclaimTask` has no foreground check off Windows; `defocus_all` blurs the
panes of every window), a hidden or closed pane keeping or regaining the keyboard, and requests applied late.

One adapter serves both, `input_owner/views.rs`, feeding the same machine. Its events:

| Event | macOS | Linux |
|---|---|---|
| `Select` | `input_select` (§3.6) | same |
| `PanePointerDown` | left mouse-down on a pane overlay (`swizzled_nsapp_send_event`) | none observed: the frontend's `Select` stands in |
| `AppPointerDown` | not yet: the same hook still hands key status back to the app window itself; the frontend's `Select` follows | none observed |
| `FocusGained` | CEF `on_got_focus` for a pane browser | same |
| `Activated` / `Deactivated` | CEF Views `on_window_activation_changed`, effects posted to run after CEF's own handling | same |
| `PaneLive` / `PaneShown` / `PaneCollapsed` / `PaneClosing` / `PaneGone` | live: the overlay window registered and on screen (`SetPaneBoundsViewsTask`, also on every re-show); a 0×0 rect is collapsed; close | live: the browser created; shown/collapsed: the rect applied; close |
| `Verify` | the key window: a tagged pane overlay is that pane, the window's own NSWindow the app | the last surface CEF reported focus to in that window |

Its effects go through three per-platform functions (`views::platform`): `focus_app(window)` (macOS: key status back
to that window's own NSWindow, its main browser focused, only *its* panes blurred; Linux: its main browser focused,
its panes blurred), `focus_pane(block)` (macOS: the overlay made key and the page first responder, without parking:
the machine parks; Linux: the pane browser focused) and `focused(window)` for `Verify`. Effects for a pointer event
run at once, on the UI thread inside the event, as the hook did before (the click that selects a pane is the one the
page gets); the others are posted. `PENDING_PANE_KEY` and its TTL are replaced by the machine's parked selection.

## 4. Phases

| Phase | Content |
|---|---|
| 1 | This spec; `crates/cef/src/input_owner/` pure machine with transition and property tests. No behaviour change. |
| 2 | Windows adapter: per-window registry, the events above wired in, effects run on the UI thread, `[input]` logging, `GetFocus` verify. Removes `ALLOW_BROWSER_PANE_FOCUS_ONCE`, the swallowed `WM_SETFOCUS`, the pre-CEF `WM_ACTIVATE` restore and the off-thread `SetFocus`. Adds `RaisePanes` and the collapse hand-off. Live-tested in an isolated dev build. |
| 3 | Frontend (§3.6): one `input_select` with a sequence number for every selection change; an `input-owner-changed` event back; `document.hasFocus()` no longer stands in for "a pane has the keys". |
| 4 | macOS (key window) and Linux (Views focus) adapters onto the same core (§3.7). |

## 5. Not in scope

- Which DOM element inside the app gets the caret: unchanged. That is `focusManager.ts` (the caret follows the selection,
  #4479; an opened input takes the caret, #4481; the window-focus net that waits 100 ms and yields to a click, #4958)
  and `userCaretInBlock`. The machine decides only app vs. pane.
- Browser panes as Views overlays on Windows (#4430): it would change the window tree, not the need for one owner.
