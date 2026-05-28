# Floating Pane — Resize & Maximize

**Date:** 2026-05-28
**Status:** Draft — pending review
**Issue:** TBD (file under #810 floating-pane umbrella)
**Author:** agent2
**Parent specs:**
- [`SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`](./SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md) — esp. §10 for why floaters can't use CEF Views.
- [`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`](./SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md)
- [`SPEC_FLOATING_PANE_REDOCK_2026-05-27.md`](./SPEC_FLOATING_PANE_REDOCK_2026-05-27.md)
- [`SPEC_FLOATING_PANE_REDOCK_PHASE_4A_SCOPING_2026-05-27.md`](./SPEC_FLOATING_PANE_REDOCK_PHASE_4A_SCOPING_2026-05-27.md)
- [`SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md`](./SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md) — §3.P5 names the missing window-state event this spec adds.
- [`secondary-windows-cef-views.md`](./secondary-windows-cef-views.md) — the lessons that forced native-mode floaters to hand-roll resize.
- [`ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`](../analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md) — context for JS-driven drag.

---

## 0. History — what's already been tried

Before specifying new mechanism, the prior art establishes which patterns
are proven on this codebase. This shapes every architectural choice
below.

| PR / commit | Mechanism | What it teaches |
|---|---|---|
| `secondary-windows-cef-views.md` + PR #263 (`12cb700f fix(cef): enable edge resize on secondary windows`) + PR #263's predecessor (`b4779082 fix(cef): remove WS_THICKFRAME to eliminate white resize border`) | Switch from raw native popup → CEF Views for secondary windows (new-window, tab tear-off) | CEF Views handles frameless / can_maximize / can_resize / WM_NCHITTEST routing natively via window delegate. Raw `browser_host_create_browser` + WS_POPUP **does not work for resize when WM_NCHITTEST handling lives on the outer WndProc** if the secondary path is set up the way pre-#263 secondary windows were. **But this lesson is conditional** — see PR #1082 below. |
| `SPEC_FLOATING_PANE_TEAROFF §10` (canonical) | Raw `CreateWindowExW` + `WS_POPUP \| WS_EX_TOOLWINDOW` + owner HWND + `set_as_child` CEF embed | The "no taskbar / no Alt-Tab / minimize-cascade with owner" behavior is **only** available via raw Win32 tool-window semantics. CEF Views does not expose `WS_EX_TOOLWINDOW` or owner semantics. **The floating pane therefore cannot switch to CEF Views** — the secondary-windows lesson does not transfer. |
| PR #1082 (`ce7e6ede`, `3346d52e`) + `install_frameless_resize_hook` in `agentmux-cef/src/client/wndproc.rs` | `WM_NCCALCSIZE=0` + `WM_NCHITTEST → HT{LEFT,RIGHT,...}` with a 6-CSS-px band, DPI-scaled per-HWND | This IS the working pattern for native-popup edge resize. The hook in `wndproc.rs:64-136` is the canonical reference; the floating-pane wndproc (`floating_pane.rs:336-412`) is a near-identical copy. **Resize at edges already works for floaters today.** The gap is not the OS hit-test — it's everything *after* the OS dispatches the resize (no CEF child tracking, no min-size floor). |
| PR #271 (`385dea8d`, `709854f8 fix(cef): hide thick resize border via DwmExtendFrameIntoClientArea`) | `DwmExtendFrameIntoClientArea(-1, -1, -1, -1)` to suppress the system-drawn thick frame | The 8-CSS-px resize band proposed below is *visually invisible* because of the existing DwmExtendFrameIntoClientArea call in `create_owned_popup` (`floating_pane.rs:532-548`). Without it, WS_THICKFRAME draws a chrome-bordered frame around the floater. **Do not remove this DWM call.** |
| PR #1094 (`8821c2e0`, `dc72ff77`) + `state.window_hwnds` cache in `commands/floating_pane.rs:142-145` | Label-routed IPC via `resolve_window_hwnd(state, label)` + cache-first lookup | The precedent that makes `maximize_window({ label: "floating-…" })` reach the right HWND. `find_own_top_level_window` (the legacy fallback `maximize_window` still uses) returns the first visible top-level — Z-order puts the floater on top, so a "main window" maximize accidentally hits the floater. **Fixing this is a one-line swap to `resolve_window_hwnd`** along the lines of `set_window_position` (`commands/window.rs:388-422`). |
| PR #315 (`bd68ece9 feat: double-click window header to maximize/restore (Windows)`) | JS `dblclick` listener in `useWindowDrag.win32.ts` → `invokeCommand("maximize_window")` | The proven pattern for dblclick-toggle-maximize on the main window. The floater needs the same shape but scoped to `[data-role="block-header"]` instead of `data-tauri-drag-region`. |
| `SPEC_MAXIMIZE_ZOOM_ARCHITECTURE §3.P5` ("Window maximize emits no state-change event") | (Diagnosed missing) | The companion architecture spec already named the gap: no `window-state-change` event, so frontend can't update icons or re-assert focus. This spec's `window_state_changed` event closes it for floaters; main-window can adopt the same channel later. |
| PR #1089 (`1446e75b fix(floating-pane): drop WS_CAPTION...`) | Style is `WS_POPUP \| WS_THICKFRAME` (no WS_CAPTION, no WS_MAXIMIZEBOX, no WS_MINIMIZEBOX) | WS_CAPTION reserves title-bar space even with `WM_NCCALCSIZE=0` — re-introducing it cuts off the bottom-right of the content. **Cannot add WS_MAXIMIZEBOX either** (it implies/requires WS_CAPTION to host the button). All maximize affordance must be rendered by the frontend inside `BlockFrame_Header`. |
| PR #1057 (`69f93b45 feat(layout): WxH badge at pane corner during resize`) | `ResizeObserver`-driven badge on docked panes | A natural follow-up extension for floater resize feedback (out of scope here but cited so reviewers know we already have a working badge component to reuse). |
| Magnify-vs-maximize disambiguation in `SPEC_MAXIMIZE_ZOOM_ARCHITECTURE §3.E` | Term convention | "Maximize" = OS window action; "Magnify" = pane action. This spec's "maximize" is the OS-window meaning, consistent with the codebase convention. |

**Net architectural verdict (locked):**

- Native popup stays. CEF Views is not available because of the
  no-taskbar / owned cascade requirement.
- Resize at edges already works via the existing `WM_NCHITTEST` + DWM
  pattern (PR #1082 + PR #271). This spec extends — does not replace —
  that pattern.
- Maximize follows the JS-driven main-window pattern (PR #315) for
  toggle + dblclick, with a hand-rolled work-area clamp via
  `WM_GETMINMAXINFO` (forced — `WS_POPUP` defaults to full-monitor).
- IPC routing follows PR #1094's `resolve_window_hwnd` precedent.

---

## 1. Problem

A floating pane today (`agentmux-cef/src/floating_pane.rs`) is a
`WS_POPUP | WS_THICKFRAME` Win32 popup with `WS_EX_TOOLWINDOW`. The
custom `floating_pane_wndproc` strips system chrome
(`WM_NCCALCSIZE → 0`, `WM_NCACTIVATE → 1`) and the standard pane's
`BlockFrame_Header` is the only chrome the user sees. Window drag is
JS-driven against `[data-role="block-header"]`
(`frontend/app/workspace/floating-pane-workspace.tsx`). The
`DwmExtendFrameIntoClientArea(-1)` call (PR #271 / `floating_pane.rs:
532-548`) hides the system-drawn thick frame so the resize band is
visually invisible.

Two interactions are missing or incomplete:

1. **Resize works at edges but is undiscoverable and lacks
   guard-rails.** `WM_NCHITTEST` already maps the outermost 6 CSS px
   of each edge / corner to `HTLEFT/HTRIGHT/HTTOP/HTBOTTOM/HTTOPLEFT/
   ...` (same template as `install_frameless_resize_hook` in
   `wndproc.rs:64-136`, polished for DPI by PR #1082's `3346d52e`).
   Win32 *does* drive resize when the cursor lands in the band. But:
   - The band is 6 CSS px (3 physical at 200% DPI before our DPI
     scaling, ~6 physical at 100%) — visually invisible and easy to
     miss.
   - There is no minimum size, so a floater can be resized down to a
     1×1 sliver and effectively lost.
   - The embedded CEF browser is created at the floater's *initial*
     (W, H) via `WindowInfo::set_as_child(parent_hwnd, &rect)` and
     never told to update when the outer HWND resizes — `WS_CHILD`
     does not auto-track parent size. Today the pane looks correct on
     first paint because the WS_CHILD matches the outer at creation
     (`floating_pane.rs:147-176`). After a resize, the WS_CHILD stays
     at the original (W, H) and the rendered content does not follow
     the outer rect.
   - There is no `set_window_size` IPC, so neither layout-save/restore
     nor agent-driven resize is possible. (Mirror of
     `set_window_position` in `commands/window.rs:388-422`, which
     **does** exist.)

2. **Maximize is unsupported.**
   - `maximize_window` (`commands/window.rs:138`) uses the buggy
     `find_own_top_level_window` (returns the first visible top-level
     of the process — which is a floater whenever one exists, per the
     long comment at `commands/window.rs:165-179`). So a label-less
     call from the main window's chrome would accidentally maximize a
     floater instead. Conversely, calling it FROM a floater has no
     route — the floater's frontend never sends `maximize_window`.
   - The `BlockFrame_Header` (the floater's only chrome) has no
     maximize affordance.
   - `WS_POPUP` without `WS_MAXIMIZEBOX` is maximized via `ShowWindow
     (SW_MAXIMIZE)`, but Win32's default behavior for `WS_POPUP`
     ignores the monitor work area and goes full-screen ABOVE the
     taskbar — wrong for a tool-window floater.
   - Standard double-click-the-titlebar to toggle maximize is broken:
     the title bar is the pane header, drag is JS-driven, and our
     wndproc never returns `HTCAPTION` from `WM_NCHITTEST`. So
     `WM_NCLBUTTONDBLCLK` never fires for the maximize path.
   - JS-driven drag against the maximized state is not handled — a
     drag on a maximized floater should restore it under the cursor
     (Windows-explorer-style "tear from maximized"), not move the
     maximized rect.

This spec describes the changes needed to make floating panes
resizable (visibly, with floor/ceiling and embedded-browser tracking)
and maximizable (with the correct work-area semantics, a header
affordance, and drag-from-maximized restore).

---

## 2. Goals

- Resize a floating pane by dragging any edge / corner of the outer
  window. CEF child resizes in lockstep. Min size enforced. No max
  size beyond the destination monitor's work area.
- Maximize a floating pane to the work area of the monitor it is
  currently on (taskbar respected). Toggle back to the prior rect via
  the same affordance. Double-click on the pane header also toggles.
- A drag gesture on a maximized floater restores it to its prior size
  and positions it under the cursor (Windows-explorer "tear from
  maximized").
- Layout persistence: the restored rect after a maximize survives
  process restart for the duration of the floater's session. (No
  long-term persistence in this phase — floaters do not survive
  AgentMux restart today.)
- No regression in: header-drag, redock-on-drop, close-X, auto-close-
  on-empty-tab, focus-ring suppression, no-taskbar / no-Alt-Tab.

## 3. Non-goals

- macOS / Linux floating-pane support. (Tracked under the
  cross-platform spec — none of the above ships on those platforms in
  this phase.)
- Min/maximize buttons inside `BlockFrame_Header` for the **docked**
  case. The affordance is conditional on the floating shell only.
- Snap-to-half (Win+Left / Win+Right). Out of scope for this phase;
  WS_POPUP doesn't pick up DWM snap layouts and a hand-rolled snap
  service is unjustified until the basic maximize works.
- Multi-monitor maximize across all monitors. Always one monitor's
  work area.
- Persistence across AgentMux restart. (No floater state survives
  today; this spec does not change that.)

## 4. UX

### 4.1 Resize

- Cursor over the outer 6 CSS-px edge band → standard system resize
  cursor (Win32 provides this from the `HT*` return value; no
  frontend work needed). Edge bands enlarged to **8 CSS px** as part
  of this phase to improve discoverability while staying inside the
  pane header's apparent boundary. Bumping further (e.g. 12 px)
  starts to occlude pane-header buttons at narrow widths.
- Drag → outer window resizes; CEF child (`set_as_child` browser HWND)
  resizes in lockstep, so the pane content follows. SolidJS
  layout reacts via the existing CEF `OnSize` → window resize event
  the docked workspace already handles.
- Minimum size: `(W_min, H_min) = (320, 180)` CSS px. Both axes
  clamped independently inside `WM_GETMINMAXINFO`. Rationale:
  - `BlockFrame_Header` is 33 CSS px (`--header-height`,
    `theme.scss:97`) with at minimum the title + close button —
    ~280 CSS px before truncation.
  - 180 CSS px tall is the smallest agent-view height where the
    composer + one log line is still readable.
- No maximum size cap beyond what the monitor's work area allows.

### 4.2 Maximize toggle

The floater's `BlockFrame_Header` grows two new buttons, immediately
left of the close-X, **only when rendered inside the floating shell**:

```
┌─────────────────────────────────────────────────────────┐
│ ⌬ agent / asaf-laptop                          ▭ ▭ ✕   │   ← header
├─────────────────────────────────────────────────────────┤
│                                                         │
│   < block contents (CEF child fills here) >             │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

- `▭` (restore icon, left of `✕`): toggles maximize. When already
  maximized, the icon switches to a restore glyph (two overlapping
  rectangles).
- The new buttons live in the existing `BlockFrame_Header` right-side
  end-icons slot (same place `magnify`, `mic`, etc. live). They are
  guarded by a `frame: "floating"` prop that the floating shell
  passes through and the docked shell does not, so docked panes are
  unaffected.

Other toggles:

- **Double-click on the pane header** (anywhere not on an interactive
  child) toggles maximize. Implementation in
  `floating-pane-workspace.tsx`'s existing capture-phase mousedown
  listener: a `dblclick` listener on the same `HEADER_SELECTOR` with
  the same `INTERACTIVE_SELECTOR` skip. Fires `maximize_window
  ({ label })`. `preventDefault` on `dblclick` is not required — it
  doesn't trigger HTML5 drag.

- **Drag-from-maximized restore**: in the existing mousedown handler
  in `floating-pane-workspace.tsx`, if `get_window_state({ label })`
  returns `"maximized"`, the handler:
  1. Reads the floater's *restored* size from a host-side stash (see
     §5.3).
  2. Computes the post-restore origin so the cursor stays at the same
     *relative* position inside the header (mirrors Windows-explorer:
     centered-ish on cursor, never off-screen).
  3. Calls `restore_window_and_move({ label, x, y, w, h })`.
  4. Continues the existing drag loop with the restored rect as the
     baseline (`initWinX/Y` are the post-restore origin, not the
     maximized origin).

### 4.3 Interactions with redock-on-drop

A maximized floater that gets dragged → restored → moved → released
over another agentmux window still redocks per
`SPEC_FLOATING_PANE_REDOCK_2026-05-27`. The restore happens BEFORE the
drag loop, so the `mouseup` redock probe sees the restored geometry
(which is what the target window needs for layout-position resolution
anyway).

A maximized floater released without dragging (header double-click →
maximize → mouseup) does NOT trigger redock — the `wasDragging` guard
in `floating-pane-workspace.tsx:267` already covers this.

### 4.4 Re-dock disables maximize

When a floater is redocked into another window, the new docked block
no longer has access to maximize (the floating shell isn't
rendering). The maximized-state stash for that floater label is
dropped along with the rest of its `window_meta` cleanup.

---

## 5. Implementation

### 5.1 Resize — host (`agentmux-cef/src/floating_pane.rs`)

**Window styles.** No change to `WS_POPUP | WS_THICKFRAME`.
`WS_THICKFRAME` is the resize-border style — keep it. Adding
`WS_SIZEBOX` is the same flag and not needed. Do NOT add
`WS_OVERLAPPEDWINDOW` — that would re-introduce the system caption
that PR #1089 stripped. Do NOT add `WS_MAXIMIZEBOX` either; it
implies a system caption to host the maximize button. We render the
maximize affordance ourselves in the header (§5.4).

**`WM_NCHITTEST`.** Bump `RESIZE_BORDER_CSS` from `6` to `8`. The
existing DPI-scaling math stays. No other change.

**`WM_GETMINMAXINFO`.** New branch in `floating_pane_wndproc`:

```rust
WM_GETMINMAXINFO => {
    // Scale (320, 180) CSS px → physical px using THIS HWND's DPI.
    let mmi = &mut *(lparam as *mut MINMAXINFO);
    let dpi = GetDpiForWindow(hwnd).max(96);
    let min_w = (320 * dpi as i32 / 96).max(1);
    let min_h = (180 * dpi as i32 / 96).max(1);
    mmi.ptMinTrackSize.x = min_w;
    mmi.ptMinTrackSize.y = min_h;
    // ptMaxTrackSize / ptMaxSize: leave defaults. Win32 caps to
    // the virtual screen, which is what we want.
    return 0;
}
```

**CEF child tracking.** In `WM_SIZE`, post a `SetWindowPos` against
the embedded browser HWND so the WS_CHILD follows the outer:

```rust
WM_SIZE => {
    // wparam: SIZE_RESTORED | SIZE_MAXIMIZED | SIZE_MINIMIZED | ...
    if wparam != SIZE_MINIMIZED {
        let w = (lparam & 0xFFFF) as i32;
        let h = ((lparam >> 16) & 0xFFFF) as i32;
        if let Some(child) = first_cef_child_hwnd(hwnd) {
            SetWindowPos(
                child, std::ptr::null_mut(),
                0, 0, w, h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
```

`first_cef_child_hwnd` walks `GetWindow(hwnd, GW_CHILD)` and picks the
first child — there's only one (the CEF browser embed). Returning
`Option<HWND>` keeps the fallthrough safe if the message fires before
the child is created (e.g. during the initial `WM_SIZE` between
`CreateWindowExW` and `browser_host_create_browser`).

The CEF browser's `OnSize` then drives the standard
`WasResized()`-style repaint and emits the resize event the SolidJS
workspace already handles, so the layout reacts.

### 5.2 Resize — backend / IPC

New IPC: `set_window_size({ label, w, h })`. Companion to
`set_window_position`. Resolves HWND via the same `resolve_window_hwnd`
path so the floater's outer is found via `window_hwnds` cache (the
fix from PR #1094). Uses the same `SWP_NOZORDER | SWP_NOACTIVATE`
flags. Width / height in CSS px — host scales to physical inside the
call using the destination monitor's DPI (same `MonitorFromWindow` →
`GetDpiForMonitor` pattern as `commands/floating_pane.rs:115-137`).

Wired into the same `Service` registry as `set_window_position`. Used
by:
- Programmatic resize from agents (future).
- The post-redock layout-restore path (out of scope for this phase
  but the IPC is the prerequisite).
- Tests / dev-tools.

NOT used by the user-drag-the-edge path. That goes straight through
`WM_NCHITTEST`'s OS-native resize loop — no IPC round-trip.

### 5.3 Maximize — host

**Label-routed `maximize_window`.** `commands/window.rs:138-162`
already exists but uses `find_own_top_level_window`. Replace with
`resolve_window_hwnd(state, label)` (same routing as
`set_window_position`). Args: `{ label }`. When label is omitted /
"main", behavior is unchanged (since the reducer registry path
resolves "main" to the actual main window's outer HWND). When label
is a `floating-…`, the cache-first `resolve_window_hwnd` returns the
floater outer.

**Restore-on-toggle.** `GetWindowPlacement(hwnd).showCmd ==
SW_MAXIMIZE` → `ShowWindow(SW_RESTORE)`. Else → `ShowWindow
(SW_MAXIMIZE)`. The existing code already does this; only the HWND
resolution changes.

**Work-area maximize.** Add `WM_GETMINMAXINFO` handling for the
maximized case too. Without it, `WS_POPUP` maximize ignores the
taskbar:

```rust
WM_GETMINMAXINFO => {
    // ... ptMinTrackSize as above ...

    // Clamp the maximized rect to the work area of the monitor the
    // floater is currently on. `MonitorFromWindow` chooses the
    // monitor with the largest intersecting rect, which is the
    // correct one as the floater is dragged across monitors.
    let mi = monitor_info_from_window(hwnd);
    let work = mi.rcWork; // taskbar-excluded
    mmi.ptMaxPosition.x = work.left - rect_of(hwnd).left;
    mmi.ptMaxPosition.y = work.top  - rect_of(hwnd).top;
    mmi.ptMaxSize.x = work.right  - work.left;
    mmi.ptMaxSize.y = work.bottom - work.top;
    return 0;
}
```

`ptMaxPosition` is relative to the window's *current* position
(Win32 quirk). The deltas above compute the offset so the maximized
top-left lands at `work.{left,top}`.

**Restored-rect stash.** Before `ShowWindow(SW_MAXIMIZE)`, host
captures the current outer rect into a new `state.floating_restored_
rects: Mutex<HashMap<String, RECT>>`, keyed by window label. On
`ShowWindow(SW_RESTORE)`, the stash entry is consumed and the
outer is `SetWindowPos`'d to that rect.

The `restore_window_and_move` IPC (§4.2 drag-from-maximized) reads
the stash, returns the rect to the frontend (or atomically restores
+ moves on the host side), and clears the stash entry.

Drop the stash entry in the floater's `WM_DESTROY` handler so a
re-used label (theoretical, since labels are UUID-tagged) cannot
inherit a stale rect.

### 5.4 Maximize — frontend

**Header affordance.** `frontend/app/element/blockframe.tsx` (or
wherever `BlockFrame_Header` lives) takes a new prop:

```ts
interface BlockFrameHeaderProps {
    // ... existing ...
    floating?: { windowLabel: string; isMaximized: Accessor<boolean> };
}
```

When `floating` is set, the header renders a maximize button (icon:
`▭` or maximize glyph; switches to restore glyph when `isMaximized()`
is true) immediately left of the close-X. `onClick` →
`invokeCommand("maximize_window", { label: floating.windowLabel })`.

Only the floating shell passes the prop; docked panes get
`undefined` and render exactly as today.

**`isMaximized` signal.** `FloatingPaneWorkspace` owns the signal,
subscribed to a new `window_state_changed` IPC event the host emits
when `WM_SIZE` fires with `SIZE_MAXIMIZED` / `SIZE_RESTORED` for a
known floater label. Alternatively (lighter-weight, no new event
channel needed): poll `get_window_state({ label })` on a `setInterval`
of 250 ms while the floater is mounted. **Pick the event-driven
path** — polling burns IPC and renders unnecessarily.

The new `window_state_changed` event is emitted by the same
`floating_pane_wndproc.WM_SIZE` branch that drives CEF-child resize:
when `wparam` is `SIZE_MAXIMIZED` or `SIZE_RESTORED`, host-dispatch
`HostCommand::EmitWindowState { label, state }`. Reducer broadcasts
to the matching browser via the existing per-window event bus.

**Double-click maximize.** In `floating-pane-workspace.tsx`'s
existing `onMount`, add a `dblclick` listener mirroring the
`mousedown` handler's selectors. `e.detail === 2` is sufficient; no
need for a manual click-counter.

**Drag-from-maximized.** Inside the `onMouseDown` handler, between
the `e.preventDefault()` and the `get_window_position` call:

```ts
const state = await invokeCommand<"normal" | "maximized" | "minimized">(
    "get_window_state", { label });
if (state === "maximized") {
    // Compute the post-restore cursor anchor: header center, but
    // never off the work area.
    const restoredRect = await invokeCommand<RestoredRect>(
        "consume_restored_rect", { label });
    const dpr = window.devicePixelRatio || 1;
    const restoredX = Math.round(e.screenX * dpr) - Math.round(restoredRect.w / 2);
    const restoredY = Math.round(e.screenY * dpr) - 16; // ~header offset
    await invokeCommand("restore_window_and_move", {
        label,
        x: restoredX, y: restoredY,
        w: restoredRect.w, h: restoredRect.h,
    });
    // Drag baseline = the restored rect we just placed.
    initWinX = restoredX;
    initWinY = restoredY;
    clickScreenX = e.screenX;
    clickScreenY = e.screenY;
    dragging = true;
    return;
}
```

The `get_window_state` IPC returns the floater's outer
`GetWindowPlacement().showCmd` mapped to a friendly string.

### 5.5 New / changed IPCs

| Command | Direction | New / Changed | Description |
|---|---|---|---|
| `set_window_size` | FE→host | **New** | Resize outer to (w, h) in CSS px, label-routed. Mirror of `set_window_position`. |
| `maximize_window` | FE→host | **Changed** | Now label-routed via `resolve_window_hwnd`. Args gain `label`. Behavior on `label="main"` unchanged. |
| `get_window_state` | FE→host | **New** | Returns `"normal" \| "maximized" \| "minimized"` from `GetWindowPlacement`. Used by floater header to render the right glyph and by the drag-from-maximized branch. |
| `consume_restored_rect` | FE→host | **New** | Returns the floater's pre-maximize outer rect AND clears the stash entry. Caller is then responsible for `SetWindowPos`-ing the floater (typically via `restore_window_and_move`). |
| `restore_window_and_move` | FE→host | **New** | Atomic: `ShowWindow(SW_RESTORE)` followed by `SetWindowPos(x, y, w, h)`. Drops the need for two IPC round-trips during drag-from-maximized. |
| `window_state_changed` | host→FE | **New event** | Fires when a floater outer transitions between normal / maximized / minimized. Carries `{ label, state }`. |

### 5.6 Reducer / state additions

```rust
// agentmux-cef/src/state.rs
pub struct AppState {
    // ... existing ...
    /// Floater label → outer rect captured just before SW_MAXIMIZE.
    /// Cleared on SW_RESTORE, WM_DESTROY, or successful redock.
    pub floating_restored_rects: Mutex<HashMap<String, RECT>>,
}

// agentmux-cef/src/reducer.rs
pub enum HostCommand {
    // ... existing ...
    EmitWindowState { label: String, state: WindowState },
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum WindowState { Normal, Maximized, Minimized }
```

### 5.7 Saga / persistence

No saga changes. No persistence beyond the in-process
`floating_restored_rects` map. Floater state does not survive process
exit today (per `floating_pane.rs:32-37`) and this spec does not
change that.

---

## 6. Edge cases & failure modes

| Case | Behavior |
|---|---|
| `WM_GETMINMAXINFO` fires before the floater has stashed a rect | Use the current `GetWindowRect` as the implicit baseline. First maximize never has a stashed restore target. |
| User drags a floater between monitors mid-resize | `WM_GETMINMAXINFO` re-reads `GetDpiForWindow` each call → min/max info follows the destination monitor. |
| Maximize on a multi-monitor setup, monitor is hot-unplugged | Win32 fires `WM_DISPLAYCHANGE`. The floater stays maximized; on next `WM_GETMINMAXINFO` it clamps to the remaining monitor's work area. Restored-rect stash may now land off-screen — clamp on consume: if the stashed rect's center is outside `EnumDisplayMonitors`'s combined virtual screen, fall back to monitor-centered restored size. |
| Double-click on a header interactive child (close button, magnify) | Skipped by the `INTERACTIVE_SELECTOR` guard same as mousedown — does not toggle maximize. |
| Single click → drag without crossing the click→drag threshold while maximized | The mousedown handler restores before the user has moved the mouse. User effectively gets "click the header to restore". This is acceptable — explorer.exe behaves similarly when you click+release on a maximized title bar (no movement = no harm; the restore is visible feedback that the click "did something"). |
| Resize border occluded by pane-header buttons at narrow widths | The right-edge resize band (8 CSS px) sits between the close-X (right-aligned in the header) and the window's right edge. At 320 px min width the close-X is ~33 px from the right edge — comfortably outside the 8 px band. The right-edge maximize button (new) is 33 px farther left — also clear. |
| Frontend / host version skew (FE sends new commands to old host) | Host's IPC router returns "unknown command"; floater logs and silently leaves resize / maximize as no-ops. The pre-skew behavior (resize via WM_NCHITTEST works, no maximize) is the fallback. |
| User maximizes the floater, then redocks it (drag to another window) | Drag-from-maximized restores first → standard redock path. `floating_restored_rects` entry consumed by the restore. |

---

## 7. Testing

### 7.1 Manual

- [ ] Resize floater from each edge / corner; cursor changes; CEF
      child follows outer 1:1.
- [ ] Resize floater below `(320, 180)` CSS px clamps at the floor.
- [ ] Click maximize button in header; floater fills monitor work
      area (taskbar visible). Icon swaps to restore.
- [ ] Click restore button; floater returns to pre-maximize rect.
- [ ] Double-click pane header (non-interactive zone) toggles
      maximize.
- [ ] Drag a maximized floater's header; floater restores under
      cursor and follows mouse. Release over the main window →
      redocks.
- [ ] Resize floater on a 100% monitor, drag to a 200% monitor,
      maximize → fills 200% monitor's work area (not stretched 2x).
- [ ] Open two floaters; maximize one; drag/move/resize the other —
      the second is unaffected, both `floating_restored_rects` entries
      are independent.
- [ ] Close a maximized floater (close-X); no leaked
      `floating_restored_rects` entry (check `state` snapshot or
      asserted in a drop log).

### 7.2 Unit / integration

- Rust unit: `floating_pane_wndproc` returns the expected `HT*`
  codes for each (x, y) inside an 8 px band. Existing tests should
  be extended.
- Rust unit: `WM_GETMINMAXINFO` clamps `ptMinTrackSize` correctly at
  96 / 144 / 192 DPI.
- Rust integration (host harness): `maximize_window({ label: "floating-X" })`
  toggles the correct HWND in a two-window scenario (main + 1
  floater). Asserts no cross-window side effects.
- Frontend (vitest, `floating-pane-workspace.test.tsx`): dblclick on
  header dispatches `maximize_window`; dblclick on the close button
  does not.
- Frontend: drag-from-maximized branch calls `restore_window_and_move`
  before the drag loop arms; non-maximized branch skips it.

---

## 8. Rollout

Single PR, gated behind no flag. The change is additive
(`set_window_size`, `get_window_state`, etc. are new IPCs; the only
behavior change to an existing IPC is `maximize_window` gaining a
label arg, which is back-compat since label defaults to "main").

The `WM_NCHITTEST` band change from 6 → 8 CSS px is silently
shipped — no migration. If the existing 6 px is preferred for any
reason during review, this is the cheapest line to revert.

Changeset: `feat(floating-pane): resize + maximize`.

---

## 9. Open questions

1. **Maximize icon set.** The codebase uses `react-icons`-style
   imports (e.g. `IconMagnify`); the existing `BlockFrame_Header`
   icons live in `frontend/app/element/icons/`. The two new icons
   (maximize + restore) need to match the existing stroke weight.
   Confirm with design before implementing — placeholder Unicode `▭`
   in the mockup above.

2. **Header-affordance footprint.** Adding two icons (maximize +
   close) might cramp the header at the 320 px min width with a long
   block title. Should the maximize button collapse into the overflow
   menu below a certain width? Probably yes — defer to the standard
   header overflow rules (see
   `SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md` for a precedent).

3. **Maximize button on docked panes.** Current spec scopes the
   affordance to floating. Some users may want a "maximize tile
   within window" (zoom to fill the workspace) on docked panes. Out
   of scope here; would be a separate spec interacting with
   `TileLayout`.

4. **Restored-rect stash leak after process crash mid-maximize.** If
   the host process crashes between `floating_restored_rects.insert`
   and the next `SW_RESTORE`, the stash entry is gone with the
   process — no leak because the floater is gone too. No issue.

5. **Concurrent `maximize_window` calls (e.g. dblclick + button
   click within the same frame).** Each call reads `GetWindowPlacement`
   then `ShowWindow`-toggles. Two calls could race: both read
   "normal", both call `SW_MAXIMIZE`, net = maximized (idempotent;
   fine). Or both read "maximized", both call `SW_RESTORE`, net =
   restored (also fine). Two interleaved calls could read different
   states and toggle — net depends on order. Acceptable; spamming
   maximize is not a documented use case and the worst outcome is
   one extra toggle.

---

## 10. Touchpoints (file inventory)

- `agentmux-cef/src/floating_pane.rs` — wndproc additions
  (`WM_GETMINMAXINFO`, `WM_SIZE` child tracking,
  `WM_NCHITTEST` band 6 → 8), `first_cef_child_hwnd` helper.
- `agentmux-cef/src/commands/window.rs` — `maximize_window`
  re-routing via `resolve_window_hwnd`; new `set_window_size`,
  `get_window_state`, `consume_restored_rect`,
  `restore_window_and_move`.
- `agentmux-cef/src/state.rs` — `floating_restored_rects` field.
- `agentmux-cef/src/reducer.rs` — `EmitWindowState` host command,
  `WindowState` enum.
- `agentmux-cef/src/client/wndproc.rs` — wire `EmitWindowState`
  emission from `WM_SIZE`'s `SIZE_MAXIMIZED` / `SIZE_RESTORED`
  branches in `floating_pane_wndproc` (not the main-window wndproc).
- `frontend/app/element/blockframe.tsx` (or the actual
  `BlockFrame_Header` source — TBD on path) — `floating?` prop,
  maximize button render.
- `frontend/app/workspace/floating-pane-workspace.tsx` — pass
  `floating` prop down via `TabContent` / Block render; install
  dblclick listener; install drag-from-maximized branch in
  mousedown; subscribe to `window_state_changed` event.
- `frontend/app/platform/ipc.ts` (or equivalent) — register the new
  commands' TypeScript shapes.
- `frontend/app/store/services.ts` — `WorkspaceService` glue if any
  command is exposed at that layer.
- Tests as listed in §7.

## 11. References

### PRs / commits
- **PR #810 (`944aeec2`)** — Phase 1: original native popup primitive.
  The `WS_EX_TOOLWINDOW`/owner choice in §10 of the parent spec is
  what locks us out of CEF Views.
- **PR #1057 (`69f93b45`)** — WxH badge during pane resize. Reusable
  for floater resize feedback (follow-up).
- **PR #1082 (`ce7e6ede`, `3346d52e`)** — native title-bar drag +
  cross-DPI size + DPI-scaled resize border. Source of the
  `WM_NCHITTEST` band pattern in `floating_pane.rs`.
- **PR #1089 (`1446e75b`, `46c19224`)** — system chrome removal:
  drop WS_CAPTION, finalize `WS_POPUP | WS_THICKFRAME`,
  DwmExtendFrameIntoClientArea. Locks out WS_MAXIMIZEBOX
  (no caption to host it).
- **PR #1094 (`8821c2e0`, `dc72ff77`)** — `state.window_hwnds` cache;
  label-routed IPC precedent (`set_window_position`,
  `close_window_by_label`).
- **PR #1112 (`a3cc4a8f`)** — Phase 4a re-dock; introduced
  `resolve_window_at_cursor` with `exclude_label`. Used by
  drag-from-maximized → redock path.
- **PR #271 (`385dea8d`, `709854f8`)** — hide thick resize border via
  DwmExtendFrameIntoClientArea. Why the 8-px resize zone is visually
  invisible.
- **PR #263 (`12cb700f`, `ed7d6fed`)** — enable resize/maximize/minimize
  on secondary windows via CEF Views switch. The lesson that **does
  not transfer** to floaters (CEF Views can't deliver the
  WS_EX_TOOLWINDOW + owner semantics floaters need).
- **PR #315 (`bd68ece9`)** — main-window dblclick maximize via
  `useWindowDrag.win32.ts`. Template for the floater's dblclick
  handler.

### Specs / analyses
- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` — canonical
  6-phase spec; §10 forces native popup (no CEF Views).
- `docs/specs/SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md` — §3.P5
  names the missing window-state event this spec adds; §3.E term
  convention (maximize = OS window, magnify = pane).
- `docs/specs/secondary-windows-cef-views.md` — why secondary windows
  switched to CEF Views; the diagnostic ("WM_NCHITTEST blocked by CEF
  child") that turned out **not** to apply to native popups with
  `WM_NCCALCSIZE=0` + WS_THICKFRAME (else PR #1082's pattern would
  not work).
- `docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` —
  resize WxH overlay; possible reuse.
- `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` —
  context for JS-driven drag (Option B chosen over HTCAPTION); the
  same reasoning extends to JS-driven dblclick maximize.

### Code touchpoints (read these before implementing)
- `agentmux-cef/src/floating_pane.rs:336-412` — current
  `floating_pane_wndproc` to extend.
- `agentmux-cef/src/floating_pane.rs:419-554` — `create_owned_popup`
  (where DwmExtendFrameIntoClientArea is set; **do not touch**).
- `agentmux-cef/src/client/wndproc.rs:64-136` — reference impl of
  `install_frameless_resize_hook` (the template the floater's wndproc
  was copied from). Mirror new WM_GETMINMAXINFO / WM_SIZE branches in
  shape.
- `agentmux-cef/src/commands/window.rs:138-162` — current
  `maximize_window` to re-route.
- `agentmux-cef/src/commands/window.rs:388-422` — `set_window_position`,
  the label-routing template for the new `set_window_size`.
- `frontend/app/hook/useWindowDrag.win32.ts` — main-window
  `installCefDragListener` + dblclick handler; the floater's
  `onMount` should mirror it.
- `frontend/app/workspace/floating-pane-workspace.tsx:107-386` —
  existing JS-driven drag; add dblclick + drag-from-maximized
  branches *in this onMount*, do not install a separate listener.
