# Spec: Instance Panel — Floating-Pane Focus Fix, Opacity Controls, Condensed Rows

**Date:** 2026-07-11
**Author:** Agent2
**Status:** Implemented (this PR)
**Surface:** version pill (status bar) → `InstancePanel` popover
**Related:** `SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` (issue #810),
`SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01` (window alpha)

---

## 1. Problem

Clicking the version pill in the status bar (`StatusBar.tsx:95-112`) opens the
`InstancePanel`, which lists this process's windows and floating panes. Three issues:

1. **Clicking a floating pane does not bring it to the foreground.** The row's click
   handler runs, but the focus silently no-ops (root cause in §2).
2. **Floating panes have no opacity control.** Window rows get an opacity slider;
   floating-pane rows are name-only, even though the OS-level mechanism
   (`WS_EX_LAYERED` + `SetLayeredWindowAttributes`) works on any HWND.
3. **Rows are too tall.** Each window costs two rows (name row + separate opacity
   row), so a handful of windows + floaters makes the popover scroll. Name and
   opacity slider should share one line.

## 2. Root cause of the focus no-op (verified)

The floating-pane row calls `handleFocusPane(entry.label)` → `getApi().focusWindow(label)`
(`InstancePanel.tsx:159-165, 588`) → CEF IPC `focus_window`
(`agentmux-cef/src/commands/window/meta.rs:160-164`) → `post_focus_window` →
`FocusWindowTask` (`ui_tasks/window.rs:373-391`):

```rust
fn execute(&self) {
    if let Some(window) = get_window_on_ui(&self.state, &self.label) {
        window.activate();
    }
}
```

`get_window_on_ui` (`ui_tasks/mod.rs:49-54`) resolves label → CEF browser →
`browser_view.window()` — a **CEF Views** window lookup. But a floating pane is not a
Views window: it's a raw Win32 `WS_POPUP | WS_EX_TOOLWINDOW` HWND with the CEF browser
embedded via `WindowInfo::set_as_child` (`floating_pane.rs`). `browser_view.window()`
returns `None` for it, so `FocusWindowTask` falls through and does nothing. Main
windows focus fine; floaters never do.

The floater HWNDs are already tracked: `ACTIVE_FLOATER_HWNDS`
(`floating_pane.rs:78`), keyed by label (`"floating-<uuid>"`) →
`(floater_hwnd, parent_main_hwnd)`. Today it's only read by
`floater_debug_snapshot()` — there is no public label→HWND getter.

## 3. Changes

### 3.1 Focus: make `focus_window` floater-aware (backend)

In `post_focus_window` / `FocusWindowTask` (or a branch in the `focus_window` command
before posting), handle `floating-*` labels via the registry:

1. Add `pub(crate) fn floater_hwnd_for_label(label: &str) -> Option<isize>` to
   `floating_pane.rs` (next to `floater_debug_snapshot`).
2. In `FocusWindowTask::execute`, before the Views lookup:

```rust
#[cfg(target_os = "windows")]
if let Some(hwnd) = crate::floating_pane::floater_hwnd_for_label(&self.label) {
    unsafe {
        // Restore first — SetForegroundWindow on a minimized window
        // activates it without un-minimizing.
        if IsIconic(hwnd as _) != 0 {
            ShowWindow(hwnd as _, SW_RESTORE);
        }
        SetForegroundWindow(hwnd as _);
    }
    return;
}
// existing Views path for main windows
```

`SetForegroundWindow` succeeds here without `AllowSetForegroundWindow` because the
call originates from a click in this same process's foreground window. Falls through
to the existing Views path for every non-floater label — no behavior change for main
windows. (Floaters are Windows-only today, matching `floating_pane.rs`'s cfg.)

### 3.2 Opacity: floating-pane rows get the same slider (backend + frontend)

**Backend.** No change needed — verified during implementation: the reducer
(`handle_set_window_opacity`) doesn't filter labels, the Win32 apply arm resolves
HWNDs via `state.window_hwnds` (which floater creation registers into,
`floating_pane.rs`), and the srv opacity write-through already deliberately skips
labels without a `backend_window_id` (floaters have no srv `Window` row). So
`setWindowOpacity("floating-<uuid>", …)` works end-to-end today; only the UI was
missing.

**Frontend.** Reuse the existing slider (min 0.35, max 1.0, step 0.05) on floating
rows. Two adaptations:

- `dispatchWindowOpacity` events and `liveWindowOpacity` lookups are keyed by
  `windowId`; floating panes have `windowId: null` (`FloatingPaneEntry`,
  `store/global.ts:176-180`). Key the opacity store by **label** instead — labels are
  unique across both windows and floaters, and the IPC side-effect
  (`setWindowOpacity(label, value)`) already takes the label.
- **No persistence for floaters in v1.** Window opacity persists via
  `window:opacity` meta on the `WaveWindow` object (`InstancePanel.tsx:538-554`);
  floaters have no backing window object, and they don't survive an app restart
  anyway. Session-only: slider drives live IPC only, skips the
  `ObjectService.UpdateObjectMeta` branch. If floaters later gain persisted state,
  opacity joins it.

### 3.3 Condensed rows: name + opacity on one line (frontend)

Merge the separate `instance-panel-opacity-row` into the name row for **both**
sections. One row per window/floater:

```
┌─ AgentMux v0.52.4 ─────────────────────────────────────────────┐
│  …instance info (version, channel, runtime)…                   │
├────────────────────────────────────────────────────────────────┤
│ This process — 2 windows                                       │
│                                                                │
│  ● main         [this]        Opacity ▓▓▓▓▓▓▓▓▓▓ 100%          │
│  ○ research                   Opacity ▓▓▓▓▓▓▓░░░  70%          │
├────────────────────────────────────────────────────────────────┤
│ Floating panes — 2                                             │
│                                                                │
│  ◈ agent: Kimi                Opacity ▓▓▓▓▓▓▓▓▓▓ 100%          │
│  ◈ terminal                   Opacity ▓▓▓▓▓░░░░░  55%          │
├────────────────────────────────────────────────────────────────┤
│  …maintenance / footer…                                        │
└────────────────────────────────────────────────────────────────┘

Row anatomy (shared by both sections):

  ● main            [this]      Opacity ▓▓▓▓▓▓▓░░░ 70%
  ─ ─────────────── ──────      ─────── ────────── ───
  │ │               │           │       │          └─ live % readout
  │ │               │           │       └─ <input type=range> 0.35–1.0
  │ │               │           └─ label, hidden below ~360px panel width
  │ │               └─ badge, current window only
  │ └─ name — click: focus · dblclick/F2: rename (windows only)
  └─ ● current window / ○ other window / ◈ floating pane
```

Layout rules:

- Name gets `flex: 1 1 auto; min-width: 0` with ellipsis truncation; the slider
  block (`flex: 0 0 auto`) keeps a fixed width (~120px slider + 4ch value) so
  percentages align in a column across rows.
- Click/keyboard semantics are split by hit area: the name region keeps the existing
  focus/rename handlers (`InstancePanel.tsx:433-470`); the slider region keeps
  `stopPropagation` so dragging never triggers focus (`InstancePanel.tsx:518`).
- Rename mode (windows only): the input replaces the name span; the slider stays
  visible and functional (unchanged handlers).
- The slider renders only when opacity is controllable — windows: `entry.windowId`
  present (existing gate); floaters: always (label-keyed).
- The `Opacity` text label is dropped entirely at this panel's fixed 320px width —
  a `title`/`aria-label` on the slider carries the affordance; the row never wraps
  to two lines.

## 4. Files Changed

| File | Change |
|------|--------|
| `agentmux-cef/src/floating_pane.rs` | Add `floater_hwnd_for_label(label)` getter |
| `agentmux-cef/src/ui_tasks/window.rs` | `FocusWindowTask`: floater branch (restore + `SetForegroundWindow`) before Views path |
| `frontend/app/store/window-opacity-store.ts` | Key live-opacity store by label instead of windowId |
| `frontend/app/statusbar/InstancePanel.tsx` | Merge opacity slider into name row (windows), add slider to floating rows, drop meta-persist for floaters |
| `frontend/app/statusbar/_instance-panel.scss` | Inline opacity control, fixed slider column |

## 5. Testing

1. Tear a pane off into a floating window, open the version pill panel, click the
   floating row → floater comes to the foreground. Minimize the floater first →
   click restores **and** foregrounds it. Main-window rows still focus as before.
2. Drag the floater row's slider → floater becomes translucent live; release at
   100% → `WS_EX_LAYERED` is removed (`remove_window_opacity` path). Window-row
   sliders keep persisting via `window:opacity` meta; floater opacity does not
   persist across app restart (documented v1 behavior).
3. Rows render one line each: long window/floater names truncate with ellipsis and
   never push the slider off-row; percentages align vertically.
4. Rename (dblclick/F2) still works on window rows with the inline slider present;
   slider drag never triggers focus or rename on either row type.
5. Keyboard: Tab reaches name region then slider; Enter/Space on name focuses the
   window/floater; arrow keys on the slider adjust opacity without focusing.
