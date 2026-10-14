# Drag space in the window and pane headers: does it need a grip?

**Status:** active — v1 in PR #4800: the dim grip in the window drag square (§5.1), the pane grip button (§5.2), the separator after the last tab (§5.5). Next: the Move menu (§5.3).
**Date:** 2026-10-13
**Author:** AgentA (Area54)
**Trigger:** Owner request: "there is a small piece of dragable space, both on the window header and on the pane header. It is always reserved so users have a space to right click or drag. there is actually a best practice grid like icon that simulatates a sort of grip. is that the best practice solution to indicate? research best practices online, write a spect to file."
**Related:** `SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md` (the window gutter, implemented, blank by the owner's choice), `SPEC_WINDOW_DRAG_HANDLE_2026_06_06.md` (a "⠿" grip, superseded, never built), `SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` §1.2, §3.6 (the pane drag space).

## 1. Answer

**It depends on which header.**

- **Window header: the research says no grip.** Every platform guideline and every major app marks the window's drag space with *empty* space, never a glyph. **The owner chose a dim grip anyway (2026-10-13)**: "its as simple as putting the dim standin" in the slot that is already there. §5.1 is that design: quiet enough not to read as a button, with the default cursor kept.
- **Pane header: yes, a subtle grip, and make it a button.** Moving panes is an app feature. The design systems that cover in-app drag handles (Atlassian, Cloudscape, NN/g) agree on four rules: a 2×3 vertical-dot grip, low-key until the header is hovered, a ≥ 24 px target, and the grip doubling as a menu with the same moves without dragging. That last one is how WCAG 2.5.7 is met, and AgentMux doesn't meet it today: docking and tear-off are drag-only (§3.2).
- **Not ⠿.** The superseded 2026-06 spec used U+283F. It is non-standard and font-dependent; the vertical 2×3 grip is the convention (§4.5).

## 2. Today (`main` @ `cf2bb56af`)

### 2.1 Window header

- The drag space is `.tab-bar-fill` ([`frontend/app/tab/tabbar.tsx`](../../frontend/app/tab/tabbar.tsx)), `flex: 1 1 auto; min-width: var(--tab-bar-drag-gutter)` = 32 px ([`frontend/app/tab/tabbar.scss`](../../frontend/app/tab/tabbar.scss)).
- **Blank:** no icon, no hover, default cursor, only a bottom hairline. That was the owner's explicit choice in `SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md` §2.
- **It lives inside the scrolling tab strip**, so when tabs overflow it scrolls off with the last tab and is visible only at the strip's right end. That spec's §2.2 considered a fixed square outside the scroll area and chose against it.
- **Dragging** is JS-driven, not `app-region: drag`, because Chromium swallows every event, `contextmenu` included, on app-region elements. A capture-phase `mousedown` checks `data-drag-region`; after 4 px it sends `start_window_drag` and the host runs a native move loop. Double-click maximizes (`frontend/app/window/useWindowDrag.win32.ts` and the darwin/linux variants).
- **Right-click** opens `TitleBarContextMenu` ([`frontend/app/window/titlebar-context-menu.tsx`](../../frontend/app/window/titlebar-context-menu.tsx)): New Window, New Tab, Show Icons Only, Pin Widgets.
- **No accessibility markup:** a plain `div`.

### 2.2 Pane header

- The drag space is `.pane-tab-strip-drag-handle` ([`frontend/app/element/PaneTabStrip.tsx`](../../frontend/app/element/PaneTabStrip.tsx)), after the "+" button.
- **It is zero-width unless the strip overflows.** A ResizeObserver then adds `--active` (45 px). When the tabs fit, the header's own empty area is the drag space.
- **Blank,** default cursor.
- **Dragging:** the whole header (`[data-role="block-header"]`) is a pragmatic-dnd `draggable()` ([`frontend/layout/lib/TileLayout.core.tsx`](../../frontend/layout/lib/TileLayout.core.tsx)), except tabs and "+". A drag moves the pane, docks it in the layout, or tears it off into a window.
- **Right-click** ([`frontend/app/block/blockframe.tsx`](../../frontend/app/block/blockframe.tsx), [`frontend/app/block/pane-actions.ts`](../../frontend/app/block/pane-actions.ts)): Copy, Paste, Split Up/Down/Left/Right, Magnify, Close Pane, Inspect Element, Pane Color, view settings. **No move item:** docking elsewhere and tear-off are drag-only.
- **No accessibility markup.**

### 2.3 Grips elsewhere

The only grip in the app is the `grip` icon on the ≡ menu's Layouts item ([`frontend/app/window/hamburger-menu.tsx`](../../frontend/app/window/hamburger-menu.tsx)). Reorder lists (bundle order) drag the whole tile with no handle. Font Awesome Free 6.7.2 has `grip` (3×2), `grip-vertical` (2×3), `grip-lines` and `grip-lines-vertical`.

## 3. Problems worth fixing, whatever the glyph

1. **The window gutter is not always reserved.** With many tabs it scrolls away, so the window can only be dragged by the 2 px edge on Windows. Microsoft's own tabbed-title-bar guidance is that "the title bar is not completely filled with tabs".
2. **Pane moves are drag-only.** WCAG 2.5.7 (AA) requires a single-pointer, non-drag way to do anything done by dragging, and moving panes is app functionality, not user-agent behaviour.
3. **Neither space says what it is.** No tooltip, no cursor, no accessible name. For the window that's normal; for panes it hides a primary interaction.

## 4. What the research says

### 4.1 Platform guidance on title-bar drag space: empty, guaranteed

- **Microsoft:**
  - "All empty space in the title bar … should be draggable."
  - The WinUI TitleBar keeps a **min drag region** "so that the user always has a place to grab the window".
  - The TabView sample reserves an empty `TabStripFooter` with `MinWidth = 188`; the custom title bar sample reserves 48 px.
  - Right-click shows the window menu; double-click maximizes.
  - Microsoft warns that drag regions in other places are hard to discover.
- **Apple HIG:** drag the window frame or toolbar; avoid overcrowding. No grip.
- **GNOME HIG:** "Always ensure that there is some blank space in the header bar to allow it to be dragged."
- **Electron:** draggable regions swallow pointer events (the reason AgentMux drags from JS); no guidance on indicators.

None of them recommends a visible grip for moving a window.

### 4.2 Apps: a guaranteed empty strip, no indicator

- Chrome and Edge always leave an empty spot between the tabs and the minimize button, however many tabs are open; Firefox copied it and adds a "Drag space" option.
- Windows Terminal reserves an empty drag bar right of its tabs.
- iTerm2 users ask for exactly that gap.

No major app marks the title-bar drag space with a glyph or a grab cursor.

### 4.3 Moving in-app elements: a grip, quiet, keyboard-reachable, doubling as a menu

- **NN/g:** grip icons "lack universal recognition" and can be mistaken for hamburger or kebab menus. Pair them with the platform's cursors, make them reachable with Tab, and grabbable with Space.
- **Atlassian (Pragmatic drag and drop, which AgentMux's pane drag uses):**
  - A handle always visible with `cursor: grab` when dragging is the primary action.
  - Shown on hover or `:focus-within` when it is secondary.
  - The vertical drag-handle icon.
  - Hit area ≥ 24×24 px (16 px when hover-only).
  - "Make the drag handle icon into a menu button" listing the moves: their WCAG route.
- **Cloudscape:** a `drag-indicator` icon, plus required non-drag single-pointer and keyboard alternatives.
- **Material 3:** its drag handle is for sheets (a 32×4 pill in a 48 dp target); its pane `VerticalDragHandle` resizes rather than moves.
- **WCAG:**
  - **2.5.7 (AA):** a single-pointer alternative to dragging. The user-agent exemption arguably covers OS window moves, not pane moves.
  - **2.5.8:** targets ≥ 24×24 CSS px.

### 4.4 Tiling and docking apps: the whole header is the handle, the menu is the alternative

- **VS Code:** drag an editor group or view by its title area; context menus and "View: Open View" move things without dragging.
- **JetBrains:** drag the tool window header; right-click → **Move to**.
- **Visual Studio:** drag the title bar; dock guides appear while dragging.
- **Chrome DevTools:** drag the panel tab, or right-click → move.
- **Blender:** right-click a border for split, join and swap.

These use no grip; the menu is the non-drag path.

A known failure mode: a panel-header drag that moves the whole window instead of the panel (reported against the Claude desktop app). AgentMux already separates the two (`data-drag-region` vs pragmatic-dnd); keep it that way.

### 4.5 Glyphs

| Glyph | Reads as | Use |
|---|---|---|
| **2×3 dots, vertical** (FA `grip-vertical`, Material `drag_indicator`, Fluent `ReOrderDotsVertical`) | move / reorder | **this one** |
| 3×2 dots, horizontal (FA `grip`) | sheet or card handle, centred on top | no |
| ⠿ (U+283F, 2×4 braille) | depends on the font | no |
| Two or three lines (`grip-lines`, Material `drag_handle`) | sheet handle, resize splitter; easily read as ☰ | no |
| One row or column of three dots | "more" (kebab, meatball) | never for dragging |

## 5. Design

### 5.1 Window header: a dim grip in the existing drag square (owner's choice)

- **Where:** inside `.tab-bar-fill`, centred in its first `--tab-bar-drag-gutter` (32 px), right after the last tab's separator (§5.5). The square stays where it is: the existing slot the owner pointed to, inside the scrolling strip, as `SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md` built it.
- **Glyph:** the 4×3 dot grip (§5.6), 12 px, `--secondary-text-color` at opacity 0.35; 0.7 while the square is hovered.
- **Takes no events** (`pointer-events: none`): a press on the grip is a press on the fill, so dragging, double-click and right-click work exactly as before. `aria-hidden`.
- **Default arrow cursor**, as on every native title bar; no tooltip.
- Unchanged: the 32 px minimum, `TitleBarContextMenu` on right-click, double-click to maximize.

### 5.2 Pane header: a grip button in the reserved space

- **Where:** a 24×24 px button at the end of the pane's tab strip, after "+", in the space `.pane-tab-strip-drag-handle` reserves. That space was "45 px only when overflowing, else 0"; it is now exactly the button's 24 px, overflowing or not. The owner, after trying v1: "we dont need all that extra space to the right of the grip", so the 45 px overflow gutter and the strip's overflow measurement are gone.
- **Glyph:** the 4×3 dot grip (§5.6), 12 px.
- **Visibility:** moving panes is secondary to working in them, so follow Atlassian's secondary-action rule.
  - At rest: `opacity` about 0.35 of `--secondary-text-color`.
  - On header hover, `:focus-within`, or while a pane drag is in progress anywhere: 1.
  - Never fully hidden, so it can be found without hovering. **D2:** always-dim (proposed) or hover-only.
- **Cursor:** `grab` on the button and on the header's empty area; `grabbing` while dragging. The existing drop-zone overlays stay.
- **Drag:** the button sits inside the header, so pragmatic-dnd's existing `draggable()` on `[data-role="block-header"]` covers it. Make sure `canDrag` doesn't exclude it (it excludes only `.pane-tab` and `.pane-tab-strip-add`).
- **Click, Enter or Space** opens the pane's header menu (the same `handleHeaderContextMenu`), anchored to the button.
- **Tooltip:** "Drag to move this pane. Click for pane actions."
- **Accessibility:**
  - `role="button"`, `tabIndex=0`, `aria-label="Move pane / pane actions"`, `aria-haspopup="menu"`. A `div` rather than a `<button>`, so a press on it can still start the header's drag in every engine;
  - reachable with Tab;
  - focus returns to it when the menu closes.
- **v1 opens the menu** by sending the header a `contextmenu` event at the grip's position, so the grip and a right-click share one handler and one menu.

### 5.3 The non-drag way to move a pane (WCAG 2.5.7)

Add a **Move** group to the pane header menu ([`frontend/app/block/pane-actions.ts`](../../frontend/app/block/pane-actions.ts)), shown from both the grip button and a right-click:

- **Move Left / Right / Up / Down:** swap with the neighbouring pane in that direction, the layout's existing swap.
- **Move to New Window:** what a tear-off does, without the drag.
- **Move to Tab › …:** the window's other tabs, if the layout model supports moving a pane between tabs; otherwise left for later.

Each item uses the layout operations the drag already uses, so a menu move and a drag move land in the same state. Announce the result through the app's live region if it has one, and keep focus on the moved pane. **D4:** which of these ship first; Move to New Window and the four swaps are the minimum for 2.5.7.

### 5.4 What not to do

- No `grab` cursor and no tooltip in the window header; its grip stays dim and takes no events.
- No ⠿, no lines, no single row of three dots.
- No hover-only grip without the menu: a hidden handle plus a drag-only action fails 2.5.7.
- No pane-drag zone touching the window-drag square without a gap: the two must stay distinct (§4.4).

### 5.5 A separator after the last tab

Owner, 2026-10-13: "there is a missing seperator at the end of the last window tab". Every tab boundary had a `.tab-separator` except the last tab's right edge, so the last tab ran straight into the drag square. `tabbar.tsx` now renders one more `.tab-separator` after the last tab (when there is one), with the same rule as the others.

### 5.6 The glyph: 4×3 dots (owner, 2026-10-13)

After trying the 2×3 `grip-vertical` in `task dev`, the owner asked for "3x3", then "a 4x3 but joining 2": two 2×3 grips side by side. Font Awesome Free has neither (`grip` is 3×2, `grip-vertical` 2×3), so [`frontend/app/element/GripDots.tsx`](../../frontend/app/element/GripDots.tsx) draws it: an inline SVG of twelve dots, four columns by three rows on a 4-unit pitch, `currentColor`, 1em tall. Both grips use it. A wide block of dots reads as "grab here" without a single column's row-reorder connotation, and stays clear of the kebab and hamburger shapes (§4.5).

### 5.7 Edge fades on overflowing tab strips (owner, 2026-10-13)

"when the pane tabs overflow, make the right edge of the overflow a gradient fade out to indicate to the user there is more to the right", then the same for the window tab bar. Both strips hide their scrollbar, so the standard cue is a scroll shadow: the side with more tabs past it fades out (28 px), and nothing fades while every tab fits or once scrolled to that end. It is a `mask-image` gradient, not an overlay, so it works on any header colour. One helper, [`frontend/app/element/scroll-edges.ts`](../../frontend/app/element/scroll-edges.ts) (`createScrollEdges`: scroll, resize and child changes, measured at most once per frame), sets `has-more-left` / `has-more-right`; one mixin, `scroll-edge-fade` in [`frontend/app/mixins.scss`](../../frontend/app/mixins.scss), draws the fade. The pane strip and the window strip both use them.

### 5.8 The grips sit outside the scrolling strips (owner, 2026-10-13)

"in the window tabs, it gets mounted to the left of the left most widget. on pane tabs it is mounted to the left of the pane controls … so that the scroll and fade only apply to the tabs, not the grips". So:

- **Window:** `.tab-bar-grip` is a fixed 32 px drag square after `.tab-bar-scroll`, just left of the widgets (`data-drag-region="true"`; the dots take no events). The empty `.tab-bar-fill` inside the strip stays draggable, without a glyph and without its old 32 px minimum: the grip outside is the guaranteed drag square, and the minimum would make the strip overflow by its own width and fade with no tab hidden (Codex on #4808).
- **Pane:** the grip is its own component, [`frontend/app/element/PaneGrip.tsx`](../../frontend/app/element/PaneGrip.tsx), rendered by `BlockFrame_Header` just before the pane controls. The tab strip's reserved slot after the "+" (`reserveDragHandle`) is gone: the grip outside the strip now keeps a place to grab the pane however far the tabs scroll.

## 6. Decisions

| | Question | Decided |
|---|---|---|
| D1 | A pinned window drag square outside the scrolling strip? | No: the owner points to the existing slot; the grip goes there (§5.1) |
| D2 | Pane grip: always visible but dim, or only on hover? | Always, dim; full on header hover or focus |
| D3 | Window items in the title bar's right-click menu? | Not now |
| D4 | First Move items in the pane menu | Move Left/Right/Up/Down and Move to New Window (§5.3, next) |

## 7. Phases

1. **v1 (PR #4800):** the window grip (§5.1), the pane grip button (§5.2), the last tab's separator (§5.5).
2. **Next, the Move menu items (§5.3):** the WCAG 2.5.7 fix; needs the layout's swap and tear-off operations exposed to the menu.

### Tests

- **v1** (`PaneTabStrip.test.tsx`): the grip renders in the spacer with its role, `aria-label` and `aria-haspopup`; a click opens the header menu; Enter and Space do, other keys don't. By eye in `task dev`: both grips, the separator, and a drag that starts on the pane grip moves the pane.
- **Move menu:** each Move item produces the same layout as the equivalent drag.

## 8. Sources

- Microsoft: [Title bar design](https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design), [TitleBar control](https://learn.microsoft.com/windows/apps/design/controls/title-bar), [TabView](https://learn.microsoft.com/en-us/windows/apps/design/controls/tab-view), [Title bar customization](https://learn.microsoft.com/en-us/windows/apps/develop/title-bar)
- Apple: [HIG Windows](https://developer.apple.com/design/human-interface-guidelines/windows), [HIG Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars)
- GNOME: [HIG Header bars](https://developer.gnome.org/hig/patterns/containers/header-bars.html)
- Electron: [Custom window interactions](https://www.electronjs.org/docs/latest/tutorial/custom-window-interactions)
- Browsers and terminals: [Firefox bug 626598](https://bugzilla.mozilla.org/show_bug.cgi?id=626598) (Chrome's always-empty spot), [Windows Terminal #4744](https://github.com/microsoft/terminal/issues/4744), [iTerm2 #12608](https://gitlab.com/gnachman/iterm2/-/issues/12608)
- Drag handles: [NN/g Drag–and–Drop](https://www.nngroup.com/articles/drag-drop/), [Atlassian Pragmatic DnD design](https://atlassian.design/components/pragmatic-drag-and-drop/design-guidelines) and [accessibility](https://atlassian.design/components/pragmatic-drag-and-drop/accessibility-guidelines/), [Cloudscape drag and drop](https://cloudscape.design/patterns/general/drag-and-drop/), [Material BottomSheetDragHandleView](https://developer.android.com/reference/com/google/android/material/bottomsheet/BottomSheetDragHandleView)
- WCAG 2.2: [2.5.7 Dragging Movements](https://www.w3.org/WAI/WCAG22/Understanding/dragging-movements), [2.5.8 Target Size (Minimum)](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html)
- Docking UIs: [VS Code user interface](https://code.visualstudio.com/docs/getstarted/userinterface), [JetBrains tool windows](https://www.jetbrains.com/help/idea/manipulating-the-tool-windows.html), [Visual Studio docking](https://learn.microsoft.com/en-us/previous-versions/z4y0hsax(v=vs.110)), [Blender areas](https://docs.blender.org/manual/en/latest/interface/window_system/areas.html), [Chrome DevTools](https://developer.chrome.com/docs/devtools/customize)
