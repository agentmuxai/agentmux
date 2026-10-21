# REPORT — Detaching a maximized pane, and where a Detach control should go

**Date:** 2026-10-21
**Author:** Maricon@charlie
**Trigger:** the operator: *"when the window is maximized, we need a way to be able to detatch a pane. currently if i try to drag it, there is no way to detach. research online for best practices on this. we may need to use the redock scheme where a temporary outline ghost appears before the real one does. We also want another window control for detach, which would bring the total to 4, is there a better pattern there?"*
**Status:** proposed — analysis only; this report changes no code. §5 is the proposal and §6 the decisions.
**Related:** `SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` (tear-off), `SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md` (the dwell-armed redock ghost), `SPEC_FLOATING_PANE_REDOCK_BUTTON_2026_10_15.md` (the floater's Redock button), `SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16.md`, `SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md` (maximize is an overlay over the tile layout), `SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md` (the grip and the pane menu).

## 1. Summary

- **Why a maximized pane can't be detached.** Detaching is drag-only: drag the pane's header out of the window. A maximized pane can't be dragged at all: `canDrag` in `frontend/layout/lib/TileLayout.core.tsx` refuses `isMagnified()`. That rule predates the SolidJS migration (it came over from the original codebase in March) and has no recorded reason. No menu item or command detaches a pane either.
- **What other apps do.** Leaving the maximized state comes first. Windows' drag-from-maximize, Chrome's tab drag, VS Code's grid and tmux's `break-pane` all un-maximize before anything moves. The detached thing takes its *restored* size, never the maximized one. A small distance threshold separates a click from a detach. A preview shows before anything commits.
- **Recommendation for 1.** Let a maximized pane's header be dragged, as a **detach-only** gesture. While the drag is inside the window, an **outline of the window-to-be** follows the cursor, at the pane's tiled size. That is the redock scheme turned around. Releasing outside the window detaches: the maximize ends and the pane becomes a floating window at its tiled size. Releasing inside cancels, and the pane stays maximized.
- **Recommendation for 2.** Don't add a fourth caption button by default. Pop-out controls in mature apps live in the content's own menu or toolbar, not next to minimize, maximize and close. Add **Move to New Window** to the pane menu (the grip and right-click), as a command-palette command (`pane:detach`), and as a shortcut, working in every state, maximized included. If a visible button is still wanted, the consistent place is where the floating pane already has **Redock**, so the two are mirror images (§4.3).

## 2. What AgentMux does today (code read on main, 2026-10-21)

- **Tear-off.** Dragging a pane's header and releasing outside every window tears it off into a floating window (`CrossWindowDragMonitor.<platform>.tsx`, `pane-tearoff.ts`, `TearOffBlock` in srv). The new window's size is the source pane's on-screen size (`measureSourcePaneSize`, `tear-off-pool-helper.ts`). For a maximized pane that would be the whole tab area, the wrong size.
- **Maximize.** Maximize is an overlay. The pane's DOM node is moved into the magnify mount, and its tile slot keeps its place and rect in the tile layout underneath (`TileLayout.core.tsx`, single-instance magnify; `SPEC_MAXIMIZE_ZOOM_ARCHITECTURE`). So the pane's tiled size is known while it is maximized: it is the tile node's rect.
- **No detach outside of a drag.** The pane menu (`pane-actions.ts`: Split, Magnify, Close, …) has no "move to new window" item, and no `pane:detach` command exists. The way back already has an explicit control: the floating pane's **Redock** button and the `pane:redock` command (#4956).
- **The controls.** A tiled pane's header ends in Minimize, Maximize and Close, after the view's own icons and a separator (`EndIcons`, `blockframe.tsx`). A floating pane's header already has four: Always on top, Redock, Maximize, Close.
- **Native Wayland** (charlie and most Linux desktops): no drag picture follows the cursor (`wayland-drag-image.ts`, so the compositor doesn't slide a "rejected" picture back after a tear-off). No client can learn its own or another window's screen position, and the compositor places new windows. Inside the window, the page still gets drag-over positions and can draw its own previews (the drop highlight, the redock ghost).

## 3. What other products do

From a web research pass with sources (vendor documentation and source code). Claims that couldn't be verified are marked.

### 3.1 Dragging out of a maximized or zoomed state

| Product | What happens | Source |
|---|---|---|
| Windows (Aero Snap) | Dragging a maximized window's title bar restores it to its earlier size, following the cursor; a registry value turns it off | winaero.com/how-to-disable-dragging-of-maximized-windows-in-windows-10 |
| Chrome / Edge tab drag | The detached window takes the source window's *restored* size, clamped to the screen with an inset, and at least half the maximized size; not re-maximized after the drop on Windows, Linux or macOS ("keeping maximized state breaks snap"); a 15 px detach threshold around the tab strip | chromium `tab_drag_controller.cc` |
| VS Code | Editors drag out to new windows (1.85); every grid change (`addView`, `moveView`, …) first calls `exitMaximizedView()` | code.visualstudio.com/updates/v1_85; vscode `gridview.ts` |
| JetBrains IDEs | A tab detaches by dragging out, Shift+F4 or Alt+drag; "Dock" brings it back; tool windows have Float/Window view modes in their Options menu | jetbrains.com/help/idea/using-code-editor.html, …/viewing-modes.html |
| tmux | `break-pane` unzooms first, then breaks out; the new floating `break-pane -W` refuses while zoomed and uses its previous floating size (master, may not be released yet) | tmux `cmd-break-pane.c`, `tmux.1` |
| WezTerm / Kitty / Zellij | Zoom plus a keyboard "move to new window" (`PaneSelect` `MoveToNewWindow`; Kitty `detach_window new-os-window`; Zellij `BreakPane`); Kitty also drags tabs out | wezterm.org, sw.kovidgoyal.net/kitty/actions, zellij.dev |
| i3, macOS tiling | Drop indicators of where the result will land, before release | i3wm.org/docs/userguide.html; support.apple.com/guide/mac-help/mchlef287e5d |

Recurring: restore first; the detached thing has its restored or previous size, clamped, with a minimum; a distance threshold; a preview before commit. Not confirmed: that Windows keeps the grab point proportional across the title bar. Chrome keeps the cursor's offset from the drag start.

### 3.2 A pop-out control

| Product | Where pop-out lives | Source |
|---|---|---|
| Microsoft Teams | An "open in new window" icon on hover, also under More options, and double-click | support.microsoft.com (Pop out a chat in Teams) |
| New Outlook | "Open in new window" at the end of the message action bar, and under More actions › View | support.microsoft.com (Pop out an email message) |
| Chrome DevTools | "Undock into separate window" in the kebab menu next to the dock-side choices; Command Menu; Ctrl/Cmd+Shift+D back to the last dock | developer.chrome.com/docs/devtools/customize/placement |
| Firefox Picture-in-Picture | A toggle shown on hover, movable by right-click | support.mozilla.org (Picture-in-Picture) |
| Slack huddles | A pop-out button opening a mini window | slack.com/help/articles/4402059023635 |
| Windows 11 Snap Layouts | A hover flyout on the window's maximize button (Win+Z too) | learn.microsoft.com (apply snap layout menu) |
| macOS Sequoia | A hover menu on the green button (Move & Resize, Arrange, Full Screen), also in Window › Move & Resize | support.apple.com/guide/mac-help/mchl9674d0b0 |
| Fluent title bar guidance | Caption buttons are minimize, maximize/restore and close, "always fully visible"; extra content (tabs, search) may sit in the title bar, but not extra caption buttons | learn.microsoft.com/windows/apps/design/basics/titlebar-design |

Icons that exist for it: VS Code codicon `open-in-window`, Fluent `Window Arrow Up` / `Window New`, Material `open_in_new`, `picture_in_picture_alt`.

Recurring: pop-out sits in the content's own header, toolbar or "…" menu, not in the caption cluster. It comes in layers: a visible or hover affordance or a menu item, plus a command and a shortcut, plus drag as the power path. Hover-only controls have a menu fallback. Every product has the reverse action. The OS already owns a hover menu on the *window's* maximize (Windows, macOS), so a hover menu on our pane maximize risks being read as that. NN/g: hidden gestures should add to visible controls, not replace them (nngroup.com/articles/contextual-swipe).

## 4. Options

### 4.1 Detaching a maximized pane by dragging

**A. Restore, then drag as usual.** On drag start, end the maximize, then the drag is an ordinary pane drag: in-tab drop targets and tear-off. This is Windows/VS Code behaviour.
- For: one drag model, and a maximized pane can also be moved within the tab.
- Against: the whole layout snaps back under the cursor at the moment the drag starts, so the pane jumps away from the grab point. The HTML5 drag has already started on the maximized element, so its geometry changes mid-drag. On Wayland there is no drag picture to bridge the jump.

**B. Detach-only drag, with an outline of the window-to-be (recommended).** The pane stays maximized during the drag. Past a threshold of about 10 px, an outline at the size the floating window will have follows the cursor inside the window: the pane's tiled size, from its tile slot, clamped to a minimum. That is the redock ghost turned around: it shows what is about to appear before the real window does.
- Release outside the window: detach. The maximize ends in the same step, and the floating window opens at the tiled size.
- Release inside the window: cancel. The pane stays maximized and nothing moves. A maximized pane covers the other panes, so there is nothing to show an in-tab drop target on.
- For: no jump under the cursor; the outline answers "how big will it be"; it matches "restore first, restored size, threshold, preview" from §3.1.
- Against: a maximized pane can't be moved within its tab by drag. Restore first for that, as today.
- On native Wayland: the outline shows only while the cursor is inside the window, as every in-window preview does there. Outside, nothing follows the cursor until the window appears where the compositor puts it.

**C. Arming by dwell, like redock.** As B, but the detach commits only after the cursor has rested outside the window for `REDOCK_DWELL_MS`.
- Against: outside the window the page gets no events on native Wayland, so there is nothing to measure dwell with. Elsewhere it adds delay to a deliberate gesture. Redock needs dwell because a floater is dragged over windows to *move* it; a maximized pane dragged out of its own window has no such ambiguity. Not recommended.

### 4.2 Detaching without a drag

An explicit action, needed in any case: on native Wayland drags are the fragile path, and keyboard and accessibility users need a non-drag way, as redock already has.
- **Move to New Window**, in every pane state (tiled, minimized, maximized).
- Where: in the pane menu (the grip's click and the header's right-click), as `pane:detach` in the command palette, and with a shortcut.
- Size: the pane's tiled size (for a minimized pane, the size it restores to).
- The reverse already exists: Redock and `pane:redock`.

### 4.3 A fourth control

| Option | For | Against |
|---|---|---|
| **1. A fourth always-visible button** (Detach, `open-in-window` glyph), left of Minimize | Discoverable; mirrors the floater, whose header has Redock in the same place (detach ↔ redock) | Crowds narrow headers; mature apps don't put pop-out in the caption cluster (§3.2) |
| **2. A hover or long-press menu on Maximize** (Maximize, Move to New Window) | No extra button | Hover-only is hard to discover and absent on touch; reads like the OS's own maximize flyout, which it isn't |
| **3. Menu item + command + shortcut** (§4.2) | No clutter; discoverable through the grip; the layering every product in §3.2 uses | Not visible until you open the menu |
| **4. Modifier-click on Maximize** (Shift+click detaches) | No extra button | Hidden; no vendor guidance for it |
| **5. Drag only** (today, plus 4.1 B) | Nothing to add | Not discoverable; fragile on Wayland; nothing for the keyboard |

## 5. Proposal

1. **Drag a maximized pane to detach it (4.1 B).**
   - `canDrag` accepts a maximized pane.
   - While maximized, the drag shows an outline at the pane's tiled size inside the window and offers no in-tab drop targets.
   - A release outside the window tears the pane off at its tiled size and ends the maximize.
   - A release inside cancels.
   - The tear-off measures the tile slot, not the maximized element, for every pane, so a maximized pane never opens a window the size of the whole tab.
2. **Move to New Window everywhere (4.2).** A pane menu item, `pane:detach` in the command palette, and a shortcut. It opens the floating window at the pane's tiled size and works in every state.
3. **No fourth caption button by default (4.3 option 3).** If the operator wants a visible one, use option 1: a Detach button left of Minimize on tiled panes, the mirror of the floater's Redock, with the `open-in-window` glyph, dropped first when the header is too narrow.

## 6. Decisions for the operator

| # | Question | Recommended | Alternatives |
|---|---|---|---|
| D1 | How does a drag on a maximized pane behave? | B: detach-only, outline of the window-to-be, release inside cancels | A: restore, then an ordinary drag |
| D2 | Size of the detached window | The pane's tiled size, clamped to the screen with a minimum | The maximized size; a fixed default |
| D3 | Detach without a drag | Move to New Window in the pane menu, `pane:detach`, a shortcut | Command only |
| D4 | A visible Detach control | No fourth caption button | A Detach button left of Minimize, mirroring Redock; a hover or long-press menu on Maximize |
| D5 | Should the floating window keep "maximized"? | No (as Chrome on Windows, Linux and macOS): it opens at the tiled size | Open the floater maximized |

## 7. Out of scope

- Moving a maximized pane within its tab by drag (restore first, as today).
- Detaching to a chosen screen position on native Wayland (the compositor places windows).
- Changes to redock, whose dwell gate and button stay as they are.
