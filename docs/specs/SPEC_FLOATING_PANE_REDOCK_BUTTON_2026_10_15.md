# Spec: A Redock button in the floating pane's header

**Date:** 2026-10-15
**Author:** Maricon@charlie
**Type:** Spec (one small button on top of the existing redock saga)
**Status:** implemented (#4956)
**Decisions applied:** D1 the main window's active tab, D3 bring it forward, D4 the `pane:redock` command, D6 every platform; D2 (a picker) and D5 (the tab it was torn from) are follow-ups.
**Requested by:** the operator: *"we should add a redock button in the floating pane control bar"*, reusing the new restore glyph from #4838.
**Related:** `SPEC_FLOATING_PANE_REDOCK_2026-05-27.md` (redock by dragging a floater onto a tile), `SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md` (the dwell gate for accidental docks), `SPEC_PANE_CONTROLS_TILED_RESTORE_ICON_AND_MINIMIZE_FROM_MAXIMIZED_2026_10_12.md` (the glyph).

## 1. The problem

A floating pane (a pane torn off into its own window) can go back to a tile layout in one way: drag it by its header onto a tile in a window. That has two recurring problems:

- **It docks when you don't want it to.** Dragging a floater over a window to move it docks the moment you let go. The same complaint was filed twice (2026-06-03 and again in September); the dwell and velocity gate in `SPEC_FLOATING_PANE_REDOCK_DWELL` is the second fix.
- **On native Wayland it does not exist.** Since #4905 (`docs/retro/retro-linux-wayland-tearoff-drag-2026-10-14.md` §6) a floater's header drag hands the move to the compositor, and while that runs the page gets no mouse motion and no mouse-up, so there are no hover ghosts and no drop on a window. No client can know its own or another window's screen position on native Wayland (`get_window_position` returns 0,0 and `set_window_position` is ignored), so "which window is under the floater" cannot be computed there at all. On a Wayland session a floater cannot be redocked today, by any gesture. X11/XWayland, Windows and macOS still redock by drag.
- **It is the fragile path elsewhere.** It depends on the cross-window drag machinery, and a user who just wants the pane back cannot say so.

A button gives an explicit, deliberate way back that needs no drag and no window positions. On native Wayland it is the **only** way.

## 2. How redock works today

- **The saga.** `WorkspaceService.RedockFloatingPane(blockId, sourceTabId, sourceWorkspaceId, targetTabId, targetWorkspaceId[, targetBlockId, direction, asTab])` (`crates/srv/src/server/service/tear_off.rs`, saga in `crates/srv/src/sagas/redock_floating_pane.rs`). It moves the block from the floater's tab into the target tab, queues the leaf for the target tab's layout and the delete for the source tab's, and the floater's now-empty tab closes the floater window. The optional arguments place it beside a given leaf; without them it is inserted at the layout's next free location.
- **The caller today.** `CrossWindowDropOverlay.tsx`, in the window that was dropped on: that window knows its own workspace and active tab, so it passes them as the target.
- **The floater header.** The floating pane's header is the normal pane header with an end-icon cluster: the "always on top" pin, a fixed maximize button, and close (`EndIcons` in `frontend/app/block/blockframe.tsx`; the floating label comes from the `?windowLabel=floating-…` URL parameter).
- **What is not recorded.** Nothing remembers which window or tab a floater was torn from; the saga is always given a target.

## 3. The behaviour

A floating pane's header gets a **Redock** button. Clicking it puts the pane back into a tile layout and closes the floater:

- **Where it goes: the active tab of the main window** (decision D1). That tab is always valid, the least surprising, and needs no screen positions, which native Wayland cannot supply. The pane is inserted the way a new pane is (the layout's next free location), not into a special slot.
- **The floater closes**, through the existing empty-tab watcher, exactly as after a drag redock.
- **The main window comes forward** so the user sees where the pane went (decision D3).
- **No main window, no button.** If no AgentMux window that can take a tile is open (the floater is the only window), the button is hidden, because there is nowhere to dock to.

What it deliberately does not do:

- It does not choose among several windows or tabs. That is D2 below; on native Wayland, where nothing else can choose, it matters more than elsewhere.
- It does not dock across AgentMux instances (also out of scope for the drag redock).
- It does not change the drag redock, its dwell gate or its ghost overlay.

## 4. The button

- **Icon:** the new tile glyph, `PaneRestoreGlyph` (`frontend/app/element/pane-control-glyphs.tsx`): a frame split into tiles with the pane's own tile filled. It means "back into the grid", which is what redock does, and it is already the Restore glyph for a minimized or maximized pane, so the app has one picture for returning a pane to its tile. A floater has no tile Restore of its own (its maximize is a fixed outline window and it has no minimize), so there is no ambiguity in its header.
- **Tooltip and accessible name:** "Redock" (the `IconButton`'s `title`, which is also its `aria-label`).
- **Position:** first in the floater's end icons, before the pin: `[Redock] [Always on top] [Maximize] [Close]`.
- **Shown when:** the pane is in a floater (`floatingLabel()` is set) and a main window exists (§5.1), **on every platform** (decision D6). A docked pane never shows it. The page can tell which window system it runs on (`getApi().getWindowSystem()`, cached as `isNativeWayland()` in `frontend/app/host/window-system.ts`), so showing it only where drag redock is missing is possible, but the accidental-dock complaint and the fragile drag apply everywhere.

## 5. Design

A small component, `FloatingRedockButton`, next to `FloatingAlwaysOnTopButton` (`frontend/app/block/floating-ontop.tsx`), used in the same `<Show when={floatingLabel()}>` branch of `EndIcons`.

### 5.1 Finding the target

The floater's renderer must find the main window's workspace and its active tab, and then call the saga with the floater's own block, tab and workspace as the source. The main window is the window row whose `host:label` is `main`; its `workspaceid` leads to the workspace, whose `activetabid` is the target tab. These are objects the renderer can already read (`ObjectService` / the MuxObj cache). No new RPC is needed, and none in the backend: the saga's contract is unchanged.

If the main window's row, workspace or active tab cannot be found, the button is hidden (not disabled), so it never offers a click that cannot work. A click whose saga call fails logs `RedockFloatingPane failed` and leaves the floater open, as the drag path does.

### 5.2 After the redock

The saga closes the floater by emptying its tab. The button then asks the host to bring the main window forward (the existing window-focus call), without taking keyboard focus into the pane: the user clicked in the floater and has not chosen a destination pane.

### 5.3 A command

For the keyboard and the command palette, register `pane:redock` (label "Redock floating pane") that does the same for the focused floater. It is optional (decision D4) and follows the existing `window:*` command pattern in `command-registry.ts`.

## 6. Tests

1. **Button shown:** a floater with a main window shows Redock first in its end icons, with the tile glyph and `title="Redock"`; a docked pane does not show it.
2. **Hidden:** with no main window row, or a main window with no active tab, the button is not rendered. It is shown on native Wayland, X11, Windows and macOS (the window system does not change it).
3. **Click:** calls `RedockFloatingPane` once with (the floater's block, its tab, its workspace, the main window's active tab, the main window's workspace), and then asks for the main window to come forward.
4. **Failure:** when the saga rejects, the error is logged and the button stays, nothing else changes.
5. **Not the drag path:** the drag overlay's own call (`CrossWindowDropOverlay`) is unchanged and its test still passes.

Live check on a real build, on each platform: tear a pane off, click Redock; the pane appears in the main window's active tab, the floater closes and the main window comes forward. Repeat with the main window on another tab, with an agent pane (a running agent keeps running), and on native Wayland and XWayland on Linux (the point of the button is that it needs no drag; on native Wayland it is the only way to redock).

## 7. Decisions for the owner

| | Question | Recommended |
|---|---|---|
| D1 | Where does Redock put the pane? | the active tab of the main window |
| D2 | When several windows or tabs are open, offer a choice (a long-click menu)? | not in this change; add if asked |
| D3 | Bring the main window forward after redocking? | yes |
| D4 | Add a `pane:redock` command too? | yes, it is small and gives the keyboard a route |
| D5 | Remember the tab a floater was torn from, so Redock returns it there when that tab still exists? | a follow-up: it needs a new "torn from" record at tear-off and a fallback to D1. On native Wayland it is worth more, since nothing else can choose a destination |
| D6 | Show the button on every platform, or only on native Wayland where drag redock is missing? | every platform: one header everywhere, and the drag's problems (accidental docks, fragility) are not Wayland-only |

## 8. What this spec does not cover

- Redocking to the tab it came from (D5), or to a chosen window or tab (D2).
- A floater that holds more than one block: today's drag redock moves the dragged block, and the floater's workspace is described as single-block (`redock_floating_pane.rs`). I did not check what a floater with several pane tabs does; the implementation should confirm that and, if it can happen, redock every block of the floater.
- Windows and macOS floaters: the saga and the header are shared code, but each platform's floater window differs, so the live check covers all three.
- I did not run the app for this spec; the call path and the window lookup come from reading the code. The first thing to confirm when implementing is that the main window's row, workspace and active tab are readable from a floater's renderer.
