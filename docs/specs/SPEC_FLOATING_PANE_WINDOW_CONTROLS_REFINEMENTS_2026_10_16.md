# Spec: Floating pane window controls: Redock placement and size, a real Restore when maximized, and drag-to-restore

**Date:** 2026-10-16
**Author:** Maricon@charlie
**Type:** Spec (refinements to the floating pane's header controls and its maximize)
**Status:** proposed
**Decisions applied (operator: "proceed with best recommendations", 2026-10-16):** D1 yes, D2 the block meta, D3 "Restore", D4 yes, D5 macOS is in this change after all: Clare on starpower checks it on a Mac, and Posa on Area54 checks Windows.
**Requested by:** the operator, after #4956: *"The new redock button on floaters should be to the immediately left of the maximize (right to the pin) ..its part of the primary. currently it appears too small, it should be the same size as the other 2 standard window controls. Second thing is that when a floating pane is maximized, we need to get proper icons ... like the 2 overlapping squares since that represents the restore in this point. we should also offer the normal restore to the maximized floater (skip minimize). Finally, when dragging a maximized floater, it should revert back to its window size before it was maximized, which is normal windows behavior."*
**Related:** `SPEC_FLOATING_PANE_REDOCK_BUTTON_2026_10_15.md` (#4956, the Redock button), `SPEC_PANE_CONTROLS_TILED_RESTORE_ICON_AND_MINIMIZE_FROM_MAXIMIZED_2026_10_12.md` (the tile Restore glyph, and where `window-restore` is still right), `SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27.md` (the pin), `SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04.md` §2.6 (restore-under-cursor for the main window), `docs/retro/retro-linux-wayland-tearoff-drag-2026-10-14.md` (Wayland floater drag).

## 1. Summary

Three refinements to a floating pane's header (a pane torn off into its own window):

1. **Redock moves and grows.** It sits immediately left of Maximize, so the window controls read `[pin] [Redock] [Maximize] [Close]`, and it renders at the same size as Maximize.
2. **A maximized floater shows Restore.** While maximized, the Maximize button becomes **Restore** with the two-overlapping-windows glyph (`window-restore`). Restore returns the floater to its size and position before it was maximized. No minimize button is added.
3. **Dragging a maximized floater restores it.** Grabbing the header of a maximized floater and dragging it returns the window to its pre-maximize size under the cursor, as Windows, macOS and GNOME do for any window.

## 2. What the code does today (main at `4e8868d26`)

### 2.1 The header

`EndIcons` in `frontend/app/block/blockframe.tsx`, for a floater (`?windowLabel=floating-…`), renders:

```
[Redock] [Always on top]* [Maximize] [Close]
```

\* The pin is Windows-only today (`canTackFloatingPane`: `hostHas("floatingAlwaysOnTop") && isWindows()`), so on Linux and macOS the row is `[Redock] [Maximize] [Close]`.

### 2.2 Why Redock looks small (measured)

The header's end-icon rule (`frontend/app/mixins.scss`, `.block-frame-end-icons .wave-iconbutton`) gives every button `width: 26px`, `padding: 4px 6px` and `font-size: 18px`. Maximize (`.block-frame-magnify`), Minimize (`.block-frame-minimize`) and Close (`.block-frame-default-close`) each reset `padding: 0`. Redock (`.block-frame-redock`) does not.

A font icon (`<i>`) ignores the squeeze and keeps its glyph size, but Redock's icon is an SVG (`PaneRestoreGlyph`, `width="1em"`), and a flex item SVG shrinks to the 14px left between the paddings, keeping its aspect ratio. Measured in headless Chrome with the header's CSS: **the Redock glyph renders 14×18 px; the same glyph with `padding: 0` renders 18×18 px**, the size of the docked Restore beside Maximize. So the ink is about 78% of Maximize's. The fix is the same `padding: 0` reset the other window controls have.

### 2.3 Floating maximize

- **The button is fixed.** `FloatingMaximizeButton` always shows `regular@window-maximize` titled "Maximize", and a click toggles. Its doc comment says this was deliberate: the reducer is the source of truth, and a frontend-mirrored state "can drift".
- **The state.** The host reducer (`crates/cef/src/reducer/pane_window.rs`, `ToggleFloatingMaximize`) flips `pane_window_states[label].placement` between Normal and Maximized. On the way up it stashes the current rect as `last_known_normal_rect`; on the way down it hands it back as `restore_rect`.
- **Persisted in the block.** `toggle_floating_maximize` (`crates/cef/src/commands/window/chrome.rs`) writes `pane:floating_placement` (`"normal" | "maximized"`) and `pane:floating_normal_rect` into the block's meta on every toggle, on every platform. The frontend types already have both keys (`frontend/types/srv-types.d.ts`).
- **The geometry differs per platform:**
  - **Windows:** floaters are borderless `WS_POPUP`s with no native maximize. The handler `SetWindowPos`es to the monitor work area on maximize, and back to the stashed rect on restore.
  - **Linux and macOS:** floaters are CEF Views windows. The handler calls `MaximizeWindowTask`, which toggles on the OS's own state (`window.is_maximized()` → `restore()` / `maximize()`), and `current_rect` is `None`, so the reducer stashes nothing.
- **What is not reported.** The module doc lists `ReportOSPlacementChange` (the OS changing placement on its own) as out of scope. Nothing tells the reducer or the block meta when the OS un-maximizes a floater.

### 2.4 Dragging a maximized floater

- **Windows:** the header drag is the host's manual move loop (`Win32BeginMoveTask`, `crates/cef/src/ui_tasks/drag.rs`). For the main window, dragging while maximized restores it under the cursor first: `begin_unmaximize_for_drag`, a 48 ms settle delay (`UNMAXIMIZE_SETTLE_MS`), then `place_restored_window_under_cursor` with `window_snap::unmaximize_drag_origin`. That keeps the grab point proportional horizontally and at the same offset vertically. **Floaters are explicitly excluded** (`source_label.starts_with("floating-")`), because they have no native maximize and `SW_RESTORE` would desync the reducer. So a maximized floater slides around at full-screen size. This is the bug the operator describes.
- **Linux:** the header drag hands the move to the compositor (`BeginWindowDrag` → `xdg_toplevel.move` on Wayland, `_NET_WM_MOVERESIZE` on X11). The floater is natively maximized there, and GNOME's Mutter un-maximizes a maximized window that is moved this way. So the window itself very likely already restores. **Not verified live.** But the reducer and `pane:floating_placement` still say "maximized", so a Restore icon driven by them would be wrong after the drag. The next button click would also send the reducer the wrong way, although the OS-side toggle would still do the right thing.
- **macOS:** the header drag is the host's own loop (`run_macos_native_drag_loop`), which moves the window with `set_bounds` at its starting size. A zoomed (maximized) floater is therefore moved at full size, the same bug as on Windows. **Not verified on a Mac.**

### 2.5 Double-click on the header

The header's `onDblClick` calls `nodeModel.toggleMagnify()`, which is the layout magnify for docked panes. In a floater, that magnifies the floater's only pane inside its own window rather than maximizing the window. On every desktop, double-clicking a title bar toggles maximize. I did not check what the magnify does visibly in a single-pane floater. It is listed as an optional decision (D4).

## 3. The behaviour

### 3.1 Redock: position and size

- **Order:** `[Always on top] [Redock] [Maximize] [Close]`. Redock sits immediately left of Maximize, as part of the primary window-control group; the pin stays outermost. Where the pin is not shown (Linux and macOS today), this is `[Redock] [Maximize] [Close]`, which is already the order there.
- **Size:** the same 26 px button with no padding as Maximize and Minimize, so the glyph renders 18×18 px, the same extent as `window-maximize` beside it (the glyph was drawn to match it, per `pane-control-glyphs.tsx`).
- Nothing else about Redock changes: same glyph, tooltip, target and hiding rule.

### 3.2 Maximize ↔ Restore

- **Normal floater:** `window-maximize` (outline square), titled "Maximize". Unchanged.
- **Maximized floater:** `window-restore` (two overlapping windows), titled "Restore". A click restores the floater to its pre-maximize size and position.
- **Why this glyph here:** the 2026-10-12 spec retired `window-restore` for *docked* panes, because docked panes never overlap and Restore there means "back into the tile". It kept `window-restore` "where a real second window is meant". A floater is a real OS window that overlaps others, so restoring it is the ordinary window restore, and the two-squares glyph is the right picture. Redock keeps the tile glyph: two different actions with two different pictures, side by side.
- **No minimize.** The floater does not get a Minimize button, maximized or not (the operator: "skip minimize").
- **Same size as Maximize:** `regular@window-restore` is a Font Awesome Free regular glyph like `regular@window-maximize`, in the same `.block-frame-magnify` button.

### 3.3 Drag a maximized floater

Pressing on a maximized floater's header and dragging past the drag threshold:

1. Restores the floater to its pre-maximize **size**.
2. Places it under the cursor so the cursor holds the same part of the header (the main window's rule, `unmaximize_drag_origin`), not at its old position.
3. Continues the drag with the restored window.
4. Leaves the floater in the Normal state: the button shows Maximize again, and `pane:floating_placement` is `"normal"`.

A click without a drag does nothing new. If there is no remembered pre-maximize size (for example, the floater was maximized before this change, or the rect could not be read), it restores to the default floater size used at tear-off.

## 4. Design

### 4.1 Redock (frontend only)

- **`blockframe.tsx`:** render `<FloatingRedockButton />` after the pin's `<Show>` and before `<FloatingMaximizeButton>`, and move the comment with it.
- **`mixins.scss`:** add `.block-frame-redock` to the `padding: 0` reset beside `.block-frame-magnify` and `.block-frame-minimize`. Better: one shared rule for the window-control class list, so the next SVG control cannot regress the same way.

### 4.2 The state the icon shows

The button needs the floater's real placement, reactively. **Recommended: the block meta `pane:floating_placement`** (D2). It is already written on every toggle on all platforms, persisted, typed, and the floater's renderer already subscribes to its block. This spec adds the two places that change placement without a button click, so the meta stops drifting:

1. **The drag restore (§4.4)** writes `"normal"` (and leaves `pane:floating_normal_rect` as it was).
2. **The OS changing it on Linux and macOS:** in `on_window_bounds_changed` for a floater window, compare `window.is_maximized()` with the reducer's placement. When they differ, dispatch a new reducer command `ReportFloatingPlacement { label, placement }` (the `ReportOSPlacementChange` the module doc already names as a later phase), and write the meta through the same fire-and-forget helper (`backend_update_block_meta`). This catches the compositor's drag un-maximize, keyboard shortcuts (Super+Up/Down), and macOS zoom from the keyboard.

The meta write goes through srv, so the icon updates a round trip after the window changes (tens of milliseconds). That is acceptable for an icon. If it proves visible, a direct host-to-renderer event can replace it later without changing the behaviour.

`FloatingMaximizeButton` becomes a reactive decl from the block's meta, like `OptMagnifyButton`: `"maximized"` gives `regular@window-restore` titled "Restore", and anything else (including a missing key) gives `regular@window-maximize` titled "Maximize". The click is unchanged (the host toggle), so a stale icon can never make a click do the wrong thing on Linux and macOS, where the toggle reads the OS state. The doc comment's "deliberately fixed" rationale is replaced by this.

### 4.3 Restore by click

No change to the host: `toggle_floating_maximize` already restores to the stashed rect on Windows, and natively on Linux and macOS. Only the icon and title are new.

### 4.4 Drag-to-restore, per platform

- **Windows (the main work).** In `Win32BeginMoveTask`, replace the blanket floater exclusion with a floater branch:
  1. If the reducer's placement for this label is Maximized, dispatch a new reducer command `RestoreFloatingForDrag { label }`. It flips to Normal and returns the stashed rect, like the toggle does, but reads the stash rather than writing one.
  2. Read the maximized rect (`GetWindowRect`), then `SetWindowPos` to the stashed rect's **width and height**. With no stashed rect, use the default floater size.
  3. Re-post with `after_unmaximize = true` and the maximized `(left, top, width)` after `UNMAXIMIZE_SETTLE_MS`, exactly like the main window. The same settle-delay reason applies: the content must re-lay-out before the nested move loop starts.
  4. `place_restored_window_under_cursor` then runs unchanged.
  5. Write `pane:floating_placement: "normal"` through to the block.

  It never calls `SW_RESTORE`, so the reducer cannot desync, which was the reason for the exclusion. The drag-to-top snap stays excluded for floaters (out of scope).
- **Linux.** Expected to need no geometry change: the compositor un-maximizes a natively maximized window on move. The work is §4.2's report, so the icon and meta follow. The live check must confirm it on GNOME Wayland and X11/XWayland. If a compositor does not un-maximize on move, the fallback is to `restore()` in the renderer's drag-threshold handler before sending `start_window_drag`. That gets no grab-point preservation on native Wayland, since no positions are known there.
- **macOS.** In `run_macos_native_drag_loop`, if the window is maximized at the first movement past the threshold, call `window.restore()` and recompute `origin` from the restored bounds, placed under the cursor with `unmaximize_drag_origin`, before the loop moves it. Then §4.2's report updates the meta. This needs a Mac to build and check: Clare on starpower (an Apple-silicon Mac) does both, and first reports what today's build does, so the change is only made if the bug is real there.

### 4.5 Double-click (optional, D4)

In a floater, the header's `onDblClick` calls the same host toggle as the button, instead of the layout magnify. Docked panes are unchanged.

## 5. Tests

**Frontend (vitest):**
1. **Order:** in a floater on Windows with the pin shown, the end icons are pin, Redock, Maximize, Close. Without the pin, they are Redock, Maximize, Close.
2. **Redock size:** the Redock button carries the class that has `padding: 0` (or the shared window-control class).
3. **Icon flip:** with `pane:floating_placement: "maximized"`, the floater's button shows `fa-window-restore` titled "Restore"; with `"normal"` or no key, `fa-window-maximize` titled "Maximize". A meta change re-renders it.
4. **Click:** either state calls `toggleFloatingMaximize(label, blockId)` once.
5. **No minimize:** a floater renders no `.block-frame-minimize`, maximized or not.
6. **Docked panes unchanged:** `OptMagnifyButton` and `OptMinimizeButton` still use the tile glyph for Restore.

**Host (cargo test, reducer):**
7. `RestoreFloatingForDrag` on a Maximized entry flips to Normal and returns the stashed rect. On a Normal entry, or one with no entry, it is a no-op that returns no rect.
8. `ReportFloatingPlacement` sets the placement and is a no-op when it matches.

**Live, per platform:**
- **Size:** compare the Redock glyph with Maximize in a screenshot of the header: same height of ink.
- **Restore by click:** maximize a floater, see the two-squares Restore, click it, and confirm the floater returns to its old size and position with the icon back to Maximize.
- **Restore by drag:** maximize, grab the header near its middle and drag. The floater shrinks to its old size under the cursor, still held near the middle, and the icon is Maximize.
- **The OS un-maximizing** (Linux Super+Down, macOS zoom shortcut): the icon follows within a moment.
- Coverage: Windows; Linux on native Wayland and X11/XWayland; macOS.

## 6. Decisions for the owner

| | Question | Recommended |
|---|---|---|
| D1 | Redock between the pin and Maximize, at Maximize's size? | yes, as asked |
| D2 | Where the icon reads the placement from | the block meta `pane:floating_placement`, plus the two new writers in §4.2; a direct host event only if the round trip shows |
| D3 | "Restore" title, or Windows' "Restore Down"? | "Restore": short, and what docked panes already say |
| D4 | Double-click on a floater's header toggles maximize, like a title bar? | yes, small and expected; it can ship separately if you'd rather keep this change tight |
| D5 | macOS drag-to-restore needs a Mac | in this change: Clare on starpower checks it on a Mac (decided 2026-10-16) |

## 7. Not covered

- Drag-to-top snap for floaters (still excluded on Windows).
- A Minimize button on floaters (the operator: skip it).
- Reopening a floater maximized after a restart, from the persisted `pane:floating_placement`.
- Redock follow-ups D2 and D5 of the Redock spec (a picker, and remembering the torn-from tab).
- The analysis comes from reading the code at `4e8868d26` and one CSS measurement in headless Chrome. I did not run a floater for this spec, and the Linux compositor's un-maximize-on-move is expected, not observed.
