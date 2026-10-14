# Retro: on native Wayland a tear-off's picture slid back, and a torn-off floater wouldn't move

**Date:** 2026-10-14
**Author:** Opaz
**Status:** implemented: on native Wayland, tear-off-capable drags have no drag
picture, and a floater's header drag hands the move to the compositor
**Severity:** Medium. Linux on a native Wayland session, which has been the
default on Wayland sessions since #1611 (2026-06-20). A torn-off pane couldn't be
moved at all; every tear-off looked like it had failed. X11/XWayland, Windows
and macOS were unaffected.

---

## 1. Symptoms

Reported on 0.59.23 (AppImage, GNOME on Wayland):

1. **A ghost after every tear-off.** Dragging a pane's header out of the window
   and releasing it opened the new floating window, but the pane's drag picture
   also slid from the release point back into AgentMux, fading as it went, the
   way a desktop shows a drop it refused.
2. **The torn-off floater wouldn't move.** Dragging its header did nothing; the
   window stayed where it opened.

## 2. Reproduction

An isolated `task dev` build of `main`, running native Wayland
(`AGENTMUX_INTERNAL_OZONE_PLATFORM=wayland`) like the AppImage, driven with real
input (ydotool) and recorded with Mutter's ScreenCast API:

1. **Tear-off.** Press on a pane's header, drag past the window's right edge,
   release. The new floater opened. In the recorded frames, the translucent pane
   picture sat at the release point, then over the next ~0.3 s (5 frames) slid
   up and left toward the source pane while fading. The pointer was still the
   whole time, and on Wayland only the compositor moves a drag's icon.
2. **Floater drag.** Press on the floater's header and drag 200 px right. In the
   page the pointer moved 190 px across the header, so the window didn't follow;
   screenshots before and after showed the floater in the same place. The host
   reported the window's position as `(0, 0)` before the drag and `(2021, 0)`
   during it: CEF's own record of the position it was asked for, which the
   compositor never applied.

## 3. Root causes

Both are the same mistake: code written and tested on X11 that relies on
something native Wayland doesn't allow.

### 3.1 The floater moved itself with `set_window_position`

On Linux the floater's header drag is JS-driven (#1314, 2026-06-11): on every
mousemove it computes a new window origin from the screen deltas and calls
`set_window_position`, which ends in `CefWindow::SetBounds`. That works on X11.
On native Wayland no client can place its own window; the compositor ignores the
origin, so the floater never moved. Wayland has exactly one way for a client to
move its window: ask the compositor to run the move (`xdg_toplevel.move`),
which the main window's title-bar drag already does through `start_window_drag`
→ `BeginWindowDrag` (`useWindowDrag.linux.ts`).

When #1314 landed, the Linux host ran on XWayland by default (`a87505603`,
2026-06-03). Nine days later #1611 switched Wayland sessions to native Wayland
after checking rendering only; the floater drag was never re-tested.

### 3.2 GNOME animates every rejected drop back

A tear-off is a drag that ends outside every window. Under Wayland's data-device
protocol nobody accepted that drop, so the compositor cancels the drag, and
GNOME's Mutter animates the drag's icon back to where the drag started (its
"drag failed" animation) before removing it.

#1186 (2026-05-30) suppressed the browser's own "drop rejected" snap-back with
pragmatic-dnd's `preventUnhandled`, which makes drops inside the page count as
handled. That can't reach a release outside every window, and on native Wayland
the animation isn't the browser's anyway. The page can't make the compositor
accept the drop, and it can't change the drag's picture once the drag has
started.

## 4. Fix

- **The host says which window system it runs on** (`get_window_system`:
  `"wayland"` for native Wayland, `"x11"` for X11 or XWayland, `"windows"`,
  `"macos"`). The page can't tell native Wayland from X11 itself. It's read once
  at startup on Linux (`frontend/app/host/window-system.ts`, from `initGlobal`)
  and answered synchronously afterwards, since drag handlers can't wait.
- **Floater drag on native Wayland** (`floating-pane-workspace.tsx`): a header
  press that moves past 4 px calls `start_window_drag`, and the compositor runs
  the move, as for the main window. A plain click still reaches the pane grip.
  X11/XWayland keeps the JS-driven drag and its redock.
- **No drag picture on native Wayland** (`frontend/app/drag/wayland-drag-image.ts`):
  pane-header, pane-tab/doc-tab and window-tab drags, every drag that can end in
  a tear-off, set a 1×1 transparent picture. The compositor still runs its
  animation, on nothing visible. The trade-off, chosen by the operator: no
  picture follows the cursor during these drags on native Wayland. Inside the
  window the drop highlight still shows where the pane will land. Other window
  systems keep their picture.

## 5. Verification

In the same native-Wayland dev build, real input:

- Before writing the fix, each remedy was tried by hand in the unfixed build: a
  transparent picture injected into the page left nothing at the release point
  across 90 recorded frames, and calling `start_window_drag` from the floater's
  header made the floater follow a 200 px × 80 px drag.
- The fixed build, the same way: `get_window_system` answered `"wayland"` and
  every window had it cached at startup. A tear-off released past the window's
  right edge opened the floater with nothing sliding back over the desktop (the
  only change near the release point was the source window reflowing). A plain
  drag of the floater's header, with no help from the test, moved the floater
  200 px right and 80 px down; the page saw no motion during the move, as
  expected while the compositor runs it.
- Not re-tested: X11/XWayland, where `get_window_system` answers `"x11"` and
  both paths are the ones from before this change.

## 6. What's still missing on native Wayland

- **Redocking a floater by dropping it on a window.** It needs the floater's and
  the windows' screen positions, which native Wayland gives no client, and the
  page sees no motion while the compositor runs the move. It didn't work before
  this fix either (the floater couldn't move).
- **Resizing a floater from its left or top edge** goes through the same
  position-setting call (`setRect`), so it likely can't move the origin there.
  Not verified in this work.

## 7. Lessons

- A switch of display backend (#1611) changes what the app may do, not just how
  it renders. Window positioning, drag-and-drop, pointer and button queries all
  differ between X11 and native Wayland; the switch needed a pass over code that
  does those things, not only a rendering check.
- When a platform can't do something (place a window, read the pointer), the
  host should say so, rather than accept the call and do nothing. Here
  `set_window_position` "succeeded" and even reported the new position back.
