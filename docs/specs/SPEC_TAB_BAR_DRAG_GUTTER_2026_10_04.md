# SPEC — Tab bar: a blank drag square between the last tab and the widgets

**Status:** implemented (#4313) — `tabbar.scss` (`--tab-bar-drag-gutter`, `.tab-bar-fill` min-width), `tab-strip-scroll.ts` and its effect in `tabbar.tsx`.
**Trigger:** owner request, 2026-10-04: "at the top, in the window tabs, we need a blank space between the last right-most tab and the left-most widget. Just a small piece, the width of a widget, but blank, needed for dragging the whole window when it is full. It can be hidden when the tabs are scrolled to the left, but when scrolling the tabs …" (the message was cut off there), then: "the width of the drag area should be the same as an unlabeled widget. Just a small square area."
**Supersedes:** `SPEC_WINDOW_DRAG_HANDLE_2026_06_06.md` (draft, never built), which proposed a grip icon on the *left*, between the hamburger and the first tab. The owner wants a blank square on the *right*, before the widgets. Mark that spec `superseded` with a pointer here when this lands.
**Scope:** `frontend/app/tab/tabbar.tsx`, `tabbar.scss` (the `.tab-bar-fill` element) and a new `tab-strip-scroll.ts`. No backend or host change: the window drag itself already works on `.tab-bar-fill`.
**Verified against:** `main` @ `decf4c21b`, 2026-10-04.

---

## 1. Problem

The header is laid out as: window controls · hamburger (Windows/Linux) · **tab strip** · action widgets · window controls.

The only empty, draggable space in the tab strip is `.tab-bar-fill`, the element after the last tab inside `.tab-bar-scroll`. It carries `data-drag-region="true"`, so a mousedown there starts a window drag (the native move loop on Windows; `isInDragRegion` in `useWindowDrag.*.ts`).

The fill is `flex: 1 1 auto; min-width: 0`. With many tabs it shrinks to **zero**, so the last tab touches the first widget and there is nowhere left to grab the window. Tabs and widgets opt out of dragging (`-webkit-app-region: no-drag` and `data-drag-region="false"`), as they should, because pressing one must select it or start a tab drag.

## 2. Design

Give the fill a **minimum width equal to one unlabeled (icon-only) action widget**, so it never collapses below a small blank square.

```scss
.tab-bar {
    // One icon-only action widget: px-2 padding either side + a text-sm icon.
    // Confirm against the live icon-only slot width when implementing.
    --tab-bar-drag-gutter: 32px;
}

.tab-bar-fill {
    flex: 1 1 auto;
    min-width: var(--tab-bar-drag-gutter);   // was 0
    // everything else unchanged
}
```

- **Nothing new in the DOM.** The fill already has the right drag attribute, the right-click behaviour (it is deliberately HTCLIENT, not `-webkit-app-region: drag`, so the title-bar context menu still opens) and the folder-boundary bottom hairline. It only stops shrinking to zero.
- **Blank.** No icon, no hover state, default cursor, same background as the strip, the same as the fill looks today. The owner asked for blank, not a grip.
- **Size.** An icon-only widget is `px-2` (8 px each side) plus a `text-sm` Font Awesome icon, about 30–32 px, and the full strip height, so it reads as a square. Use a CSS custom property, not a runtime measurement. The square does not need to track the widget to the pixel, and a fixed value avoids a measure-and-relayout loop in the strip. It scales with chrome zoom like everything else in the header.

### 2.1 Behaviour as tabs are added

| Tabs | What the user sees |
|---|---|
| Few | Unchanged: the fill is wide, and the whole empty area is draggable. |
| Enough to fill the strip | Tabs shrink a little sooner (by about 32 px in total, shared across tabs); a blank square stays between the last tab and the first widget. |
| So many that tabs hit their minimum (60 px) and the strip scrolls | The square is the **last thing in the scrolled content**. Scrolled to the right end, it shows next to the widgets. Scrolled toward the start, it is off-screen with the last tabs. That is the case the owner allowed: "it can be hidden when the tabs are scrolled to the left". |

Because the square is part of the scrolled content, it is included in the strip's `scrollWidth`, so scrolling all the way right always brings it fully into view.

### 2.2 Alternative considered: a fixed square outside the scrolling strip

Put a separate 32 px drag element *after* `.tab-bar-scroll` (a sibling inside `.tab-bar`) so it is visible however the tabs are scrolled.

- **For:** always visible, even with a scrolled strip.
- **Against:** it takes 32 px from the strip permanently and adds a second drag element. The comment in `tabbar.tsx` records that moving the fill outside the scroll container once left a dead zone inside it. Here the fill would stay inside, so that would not repeat, but there are then two drag areas to keep right.

Recommended: the design in §2. It matches what the owner allowed ("can be hidden when scrolled to the left") and is a one-line change. If the cut-off sentence meant "it must stay visible while the tabs are scrolled", switch to §2.2. See Q1.

## 3. Edge cases

| Case | Behaviour |
|---|---|
| **New tab** (Ctrl+T or "+") on a full, scrolling strip | Before this change nothing scrolled the strip to the active tab, so a new tab could open off-screen. Now, whenever the active tab changes: if it is the **last** tab, scroll the strip to its very end, so the square shows too; otherwise scroll the least amount that brings the tab fully into view (`revealTabInStrip`). It sets the strip's `scrollLeft` only, never `scrollIntoView`, which would also scroll the strip's ancestors (the page-shift bug in `REPORT_LAN_UNDISCOVERABLE_AND_PAGE_SHIFT_2026_10_04.md` §3). |
| **Reordering a tab** by dragging it over the square | It counts as "after the last tab", exactly as dropping on the fill does today: `computeInsertionPoint` (`tabbar-dnd.ts`) compares the cursor with the remaining tabs' centres only, and a cursor past every centre appends. No change needed. |
| **Tearing a tab off** and releasing over the square | Still inside the strip, so it is a reorder to the end, not a tear-off. The tear-off hit test (`tab-tearoff-rpc.ts`) uses the scroll container's rectangle, which already contains the fill. |
| **Dragging a pane onto the strip** | No drop target on the square. Same as the fill today. |
| **Double-click on the square** | Whatever double-click on the fill does today (maximize/restore through the drag hook). No change. |
| **Right-click on the square** | The title-bar context menu, as on the fill today. |
| **Wheel over the square** | Scrolls the tab strip horizontally, as anywhere in the strip. |
| **Narrow window, widgets collapsed to icons or "more"** | The square's 32 px counts in the strip's width before the widget bar decides its tier. It is part of the strip's own content, so the widget bar's measurement does not need to change. Check that it does not push the widgets into a lower tier at a width where they fit today. |
| **macOS** | The hamburger sits at the far right, after the widgets. The square still sits between the last tab and the first widget. The darwin drag hook uses the same `isInDragRegion`, so no platform branch is needed. |
| **Tab bar in its alternative position** (`SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md`) | Same element, same rule. Confirm visually in that layout. |
| **One tab only** | Unchanged: the fill is already wide. |

## 4. Tests

- `tab-strip-scroll.test.ts`: the last tab scrolls the strip to its end; a tab past either edge scrolls just enough; a visible tab and a non-scrolling strip are left alone.
- The square's width is CSS and jsdom has no layout, so it is checked by hand.
- Manual, on Windows: with the strip full, the square stays between the last tab and the first widget and drags the window; with the strip scrolling, scrolled to the end, it shows and drags; a new tab scrolls the strip to the end; right-click on the square opens the title-bar menu.

## 5. Open questions

1. **The cut-off sentence.** "It can be hidden when the tabs are scrolled to the left, but when scrolling the tabs …". This spec reads it as "…it shows when you scroll to the right end". If the owner meant "it must stay put while the tabs scroll", use §2.2 instead.
2. **Size.** 32 px is the expected icon-only widget width. Confirm it on the live header and adjust the token if it looks off beside the widgets.
