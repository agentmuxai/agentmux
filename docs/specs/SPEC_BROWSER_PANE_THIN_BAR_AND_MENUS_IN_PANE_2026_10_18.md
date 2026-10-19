# SPEC: The browser pane's thin bar puts Bookmarks on the right with a line between its rows, and its menus stay inside the pane

**Status:** active — built in #5153 (T1 and T2 together, plus §5.5); tried by the operator in a dev build on 2026-10-18.
**Author:** Korp@narko, 2026-10-18, at the operator's request
**Trigger (operator, 2026-10-18):** "in the browser pane also, 2 changes: when in responsive mode (thin) have the bookmarks button on the right of the address bar, to the left of profiles. On normal, it would be on the left. Lets also add a design line over the break on thin-view so there is some design continuouity, there should be a best practice. Second thing is that unlike other panels, the browser pane panels should stay within the bounds of the pane (as much as possible, if it is pushed out by a pane being thin, that is ok, but it should make best attempt) under normal conditions, the profile would open into the left (from the right) (as opposed to into the ight which currently happens. wirte a spec to file"
**Builds on:** `SPEC_BROWSER_PANE_NAV_BAR_RESPONSIVE_AND_CHROME_ZOOM_2026_10_14.md` (the two layouts, one DOM, the 388px breakpoint), `SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md` (the Profile menu), `SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md` (the Bookmarks menu), `SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20.md` (`computeMenuPosition`, the one positioning path every DOM menu uses).

---

## 1. Summary

1. **Bookmarks moves to the right in the thin layout.** In a thin pane the top row becomes `← → ↻ ⌂ … 🔖 👤`: Bookmarks sits just left of Profile. In the wide layout it stays where it is, on the left after Home. The DOM doesn't change; only grid areas do, as in the 10-14 spec (§3).
2. **A line between the thin bar's two rows.** In a thin pane, a 1px full-width hairline separates the button row from the address row. It uses the same colour and weight as the bar's own bottom border, so the two rows read as two bands of one bar (§4).
3. **The browser pane's menus stay inside the pane.** Bookmarks and Profile open inside the pane's rectangle when they fit, not just inside the window. Each opens from the side its button is on: Profile, always at the right, opens leftward (its right edge lined up with the button's). Bookmarks opens rightward in the wide layout and leftward in the thin one. A menu that doesn't fit in the pane (a pane narrower or shorter than the menu) may extend past it, but never past the window (§5).
4. **Only the browser pane opts in.** Every other menu in the app keeps today's behaviour: it is kept inside the window only (`avoidNativePanes: false`, window boundary).
5. **A browser pane is at least 250px wide** when a splitter is dragged (§5.5).
6. **Frontend only.** `browser-nav-bar.tsx`, `browser-profile-menu.tsx`, `browser-view.scss`, `flyoutmenu.tsx`, `menu-position.ts` and `layoutResize.ts`. No host, CEF or srv change.

## 2. What exists (verified on `main` @ `75f341fdc`, 2026-10-18)

| Piece | State |
|---|---|
| The bar | `BrowserNavBar` (`frontend/app/view/browser/browser-nav-bar.tsx`). `div.browser-nav-bar` (the container, `container: browser-nav / inline-size`, `zoom: var(--zoomfactor)`) holds `div.browser-nav-bar-grid` with three children: `.browser-nav-group--nav` (Back, Forward, Reload, Home, and the Bookmarks `FlyoutMenu`), `.browser-address-wrap`, and `.browser-nav-group--profile` (`BrowserProfileButton`). |
| Layouts | `browser-view.scss`: wide `grid-template-areas: "nav address profile"`; below `$browser-nav-stack-below: 388px` (in the bar's own px), `"nav . profile" / "address address address"`. The grid has `gap: var(--space-1)` and `padding: var(--space-1) var(--space-2)`. Nothing separates the two thin rows; the bar has `border-bottom: 1px solid var(--border-color)`. |
| Bookmarks menu | A `FlyoutMenu` with `placement="bottom-start"`, its trigger a `button.browser-nav-btn` inside the nav group. |
| Profile menu | `BrowserProfileButton` (`browser-profile-menu.tsx:144`): a `FlyoutMenu` with `placement="bottom-start"`. The button is at the right end of the bar in both layouts, so the menu's left edge is at the button and the menu runs to the right, past the pane's right edge and over the next pane, or to the window edge, where `shift()` pushes it back. That is the "opens into the right" the operator saw. |
| Positioning | `FlyoutMenu.updatePosition` (`flyoutmenu.tsx:108`) calls `computeMenuPosition({ anchor, placement, avoidNativePanes: false })` (`frontend/app/util/menu-position.ts:229`): `offset → flip → shift → size`, all with the **window** viewport as `boundary`. `autoUpdate` keeps it attached. The menu portals to `document.body` and is a pane overlay (`data-pane-overlay`), so the native page is cut away under it (Windows) or frozen (`use-freeze-frame.ts`, macOS/Linux). |
| Other browser-pane panels | The address suggestions (`.browser-address-suggestions`) are absolutely positioned inside `.browser-address-wrap`, `left: 0; right: 0`: already inside the pane. The Auto-approve confirmation and the action Log are in the pane's own flow: already inside. Home's right-click menu and the page's right-click menu go through `ContextMenuModel.showContextMenu` (a menu at the cursor), not `FlyoutMenu`. |

## 3. Bookmarks on the right in the thin layout

### 3.1 The layouts

```
Wide (bar ≥ 388px in its own px), unchanged:
┌──────────────────────────────────────────────────────────────┐
│ ← → ↻ ⌂ 🔖 [ https://example.com/path                     ] 👤 │
└──────────────────────────────────────────────────────────────┘

Thin (bar < 388px):
┌──────────────────────────────┐
│ ← → ↻ ⌂                🔖 👤 │
├──────────────────────────────┤   ← the line (§4)
│ [ https://example.com/path   ]│
└──────────────────────────────┘
```

Reading of the request: "on the right of the address bar, to the left of profiles" is taken as the thin layout's right-hand group, where Profile is, with Bookmarks immediately left of Profile. See §9 Q1 for the other reading (Bookmarks on the address row).

### 3.2 Markup: Bookmarks becomes its own grid item

The 10-14 spec's rule holds: one DOM, only grid areas change, so a resize never remounts the address input (focus, IME composition and open suggestions survive). Bookmarks moves out of `.browser-nav-group--nav` into its own grid cell, between the nav group and the address bar:

```tsx
<div class="browser-nav-bar-grid">
  <div class="browser-nav-group browser-nav-group--nav">← → ↻ ⌂</div>
  <div class="browser-nav-group browser-nav-group--bookmarks"><FlyoutMenu …>🔖</FlyoutMenu></div>
  <div class="browser-nav-break" aria-hidden="true" />      {/* §4; shown only when thin */}
  <div class="browser-address-wrap">…</div>
  <div class="browser-nav-group browser-nav-group--profile">👤</div>
</div>
```

```scss
.browser-nav-bar-grid {
    grid-template-columns: auto auto minmax(0, 1fr) auto;
    grid-template-areas: "nav bookmarks address profile";
}

@container browser-nav (max-width: #{$browser-nav-stack-below - 0.02px}) {
    .browser-nav-bar-grid {
        grid-template-columns: auto minmax(0, 1fr) auto auto;
        grid-template-areas:
            "nav     .       bookmarks profile"
            "break   break   break     break"
            "address address address   address";
    }
}
```

- The grid's 4px column gap is the nav group's own gap, so in the wide layout Bookmarks sits exactly where it sits today (4px after Home).
- **The breakpoint is unchanged.** The wide row's fixed parts are the same five 28px buttons, Profile and gaps: 208px, so 388px still leaves the address bar 180px.
- **Keyboard order** stays Back → Forward → Reload → Home → Bookmarks → address bar → Profile in both layouts. In the thin one Bookmarks is drawn on the right of the first row but reached before the address bar. This is the same trade the 10-14 spec made for Profile (§3.4 there): one order for both layouts means a resize never changes what Tab does.
- Below about 208px the nav group still wraps as a last resort; Bookmarks and Profile keep their own cells.

## 4. The line between the thin bar's rows

### 4.1 What best practice says

- Material 3's divider guidance: **full-width dividers separate sections** of a surface (and interactive areas from each other's groups); **inset dividers separate related items within one section**, aligned with an anchoring element such as an icon. Use them sparingly. ([M3 Divider guidelines](https://m3.material.io/components/divider/guidelines))
- The thin bar's two rows are two sections of one surface: a row of actions and a text entry. They aren't items of a list. So the line is **full width**: edge to edge across the bar, like the bar's own bottom border. An inset line, stopping short of the edges, would read as a separator inside one section and would look like a misaligned second border.
- **Continuity** comes from using the same token and weight as the border the bar already has: `1px solid var(--border-color)`. The bar then reads as one bar with two bands, framed by the same line at its middle and its bottom, rather than a second bar stacked under the first.

### 4.2 The rule

```scss
.browser-nav-break {
    display: none;
    grid-area: break;
}

@container browser-nav (max-width: #{$browser-nav-stack-below - 0.02px}) {
    .browser-nav-break {
        display: block;
        height: 1px;
        background: var(--border-color);
        // Full width: through the grid's side padding to the bar's edges.
        margin-inline: calc(-1 * var(--space-2));
    }
}
```

- The grid's row gap (4px) sits on either side of the line, so the rows keep their spacing and the line is centred in it. The bar gets 5px taller in the thin layout (the line plus one more row gap). `usePaneRectSync`'s `ResizeObserver` already follows a taller bar.
- A `div` and not a `border-top` on the address wrap: the line must cross the grid's padding to reach the edges, and the address wrap's own box must stay the input's width (the suggestion list is `left: 0; right: 0` of it).
- `aria-hidden`: it is decoration, not a separator a screen reader should announce.
- Chrome zoom scales it with the bar, exactly as it scales the bar's bottom border, so the two lines always match.

## 5. The browser pane's menus stay inside the pane

### 5.1 The rule

For a menu opened from the browser pane's own bar:

1. **Open from the side the button is on.** If the button is in the left half of the pane, the menu's left edge lines up with the button's left edge and the menu opens rightward (`bottom-start`). If it is in the right half, it opens leftward (`bottom-end`) with its right edge on the **pane's** right edge, not the button's (operator, after trying it: "its panel's right border should align with the pane's right border"), so Bookmarks' menu, from a button 32px in from the edge, still ends flush with the pane. Profile is always on the right, so it always opens leftward. Bookmarks opens rightward in the wide layout and leftward in the thin one, with no knowledge of the breakpoint in code: the side follows from where the button is.
2. **Stay inside the pane.** The pane's rectangle (the `.browser-view` element: bar and page, the whole pane under its header; a 4px margin at the bottom) is the boundary for `flip`, `shift` and `size`. A menu that fits is moved along the bar to fit (`shift`), and its height is capped to the room below the button in the pane, scrolling inside (`size`).
3. **Best attempt when it doesn't fit.** The pane may be thinner or shorter than the menu:
   - **Too narrow:** the menu keeps its edge on the pane's edge and runs past the other edge of the pane, never past the window. Profile's or Bookmarks' menu in a 200px pane has its right edge at the pane's right edge and overshoots to the left over the neighbouring pane, as far as it needs to, within the window.
   - **Too short:** the room below the button in the pane is capped at a floor of `min(240px, the menu's height)`. If the pane has less, the menu extends below the pane, still capped by the window, rather than becoming a two-row scroller.
   - It never flips above the bar: above the bar is the pane's own header, and a menu there would cover the button that opened it.
4. **Re-checked as things move.** `autoUpdate` already re-runs the position on scroll, resize and layout change; it now re-measures the pane too, so a menu stays inside the pane while a splitter is dragged.

### 5.2 Why the pane, and why only this pane

- The page under the bar is a native window. A DOM menu over it is shown by cutting a hole in that window (Windows) or by freezing the page under a snapshot (macOS and Linux, `use-freeze-frame.ts`). A Profile menu that ran into the next pane did that to the **next** pane too: on macOS and Linux that pane froze while the menu was open. Kept inside its own pane, a menu only ever affects the pane it belongs to.
- The pane's menus belong to the pane: a menu that stays inside it is easy to tie to its pane when several browser panes sit side by side.
- Other menus (title bar, pane header, widget bar, context menus) belong to the window. Their behaviour stays as is.

### 5.3 Changes

**`menu-position.ts`.** `MenuPositionRequest` gets two optional fields:

```ts
/** Keep the menu inside this element's rect when it fits (the browser pane);
 *  the window still bounds it when it doesn't. Default: the window only. */
containWithin?: HTMLElement;
/** With `containWithin`: pick `-start` or `-end` from which half of the
 *  container the anchor is in (§5.1 step 1). */
alignToAnchorSide?: boolean;
```

With `containWithin`:

1. Resolve the placement: `bottom-start` or `bottom-end` from the anchor's centre against the container's centre, if `alignToAnchorSide`.
2. Position with the container's rect (minus the 4px margin) as `boundary` for `shift` and `size`; `flip` is given only `fallbackPlacements: []` on the main axis (never above).
3. If the menu is wider than the container, or the room below is under the §5.1 floor, position again with the window as `boundary`, keeping the placement from step 1: the button-side edge stays anchored and the menu runs past the pane on the far side.

Without `containWithin`, nothing changes: the window is the boundary, as today.

**`flyoutmenu.tsx`.** `MenuProps` gets `containWithin?: () => HTMLElement | undefined` and `alignToAnchorSide?: boolean`, passed through in `updatePosition`. Submenus keep today's positioning: neither browser menu has any. A future one should get the same container.

**`browser-nav-bar.tsx` and `browser-profile-menu.tsx`.** Both `FlyoutMenu`s get `containWithin={() => barRef?.closest(".browser-view")}` and `alignToAnchorSide`. `BrowserProfileButton` takes the container from its caller. Whether the Profile menu also gets `mirrored` (icons on the right, labels right-aligned, as the macOS far-right hamburger menu does) is decided by trying it; the default is unmirrored, so its rows read like every other menu in the pane (§9 Q3).

### 5.5 A browser pane is at least 250px wide

**Operator, 2026-10-18:** "lets also make the min width of the browser pane 250px".

- Dragging a splitter stops a browser pane at **250px** wide (`MinBrowserPaneWidthPx`, `frontend/layout/lib/layoutResize.ts`). Every other pane keeps the 128px floor (`MinNodeSizePx`), and heights keep 128px for all.
- **Per pane, not one number.** `minExtentPx(node, axis)` gives each sibling of the dragged handle its own floor: 250px across for a pane holding a browser in any of its in-pane tabs; for a group of panes, its members' floors added up along the group's direction (with the gaps) and the widest across it. A column holding a browser pane can't be dragged narrower than 250px either. Minimized panes take no room. The floors are worked out once per drag (`ResizeContext.floors`) and used by both drag modes (the group drag and Shift's two-pane drag).
- **Same reach as the existing floor.** Like the 128px floor, it applies to splitter drags. Splitting a pane, shrinking the window and the Shift+window-edge resize don't enforce it, as they don't enforce 128px today; a browser pane can still end up narrower that way. Enforcing every floor on those paths is a separate change.
- At 250px the thin bar's buttons (about 208px with padding, §3) still fit on one row without wrapping, and the address bar keeps its own full-width row.

### 5.4 Out of scope

- **Context menus** (Home's right-click, the page's right-click): they open at the cursor through `ContextMenuModel`, which can be a native menu. Keeping them inside the pane is a separate change.
- **Chrome zoom of the menus.** `FlyoutMenu` doesn't follow chrome zoom anywhere in the app (`REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md`). Unchanged here.
- The address suggestions, the Auto-approve confirmation and the Log are already inside the pane.

## 6. Tests

### 6.1 Unit (vitest, jsdom)

- `menu-position.test.ts`, with mocked rects (as the existing tests do):
  - an anchor in the right half of a container with `alignToAnchorSide` gets `bottom-end` and its right edge on the anchor's right edge; in the left half, `bottom-start`;
  - a menu that fits stays inside the container (its rect inside the container's, with the margin);
  - a menu wider than the container keeps its anchored edge on the container's edge and stays inside the window;
  - a container shorter than the floor lets the menu extend below it, capped by the window;
  - without `containWithin`, the result is the same as today (one existing case re-run as a guard).
- `browser-nav-bar.test.tsx`: Bookmarks is its own `.browser-nav-group--bookmarks` cell after the nav group and before the address wrap; the `.browser-nav-break` element exists and is `aria-hidden`; the Tab order is unchanged.

### 6.2 Live (own `task dev`, never the operator's instance)

At bar widths 300, 387, 388 and 900px, chrome zoom 1.0 and 1.5:

- Wide: Bookmarks after Home, as before. Thin: Bookmarks just left of Profile, the line across the full width between the rows, matching the bottom border.
- Profile's menu opens leftward in both layouts, inside the pane. Bookmarks opens rightward when wide and leftward when thin.
- In a 220px pane the Profile menu runs past the pane's left edge but not the window's; in a 150px-tall pane the menu extends below it.
- Two browser panes side by side (on macOS or Linux): opening Profile in the left one doesn't freeze the right one.
- A resize while the address bar has focus, mid-typing, keeps focus and the suggestions (the 10-14 spec's guarantee).
- Screenshots of each, before and after.

## 7. Phases

| Phase | What |
|---|---|
| T1 | §3 and §4: Bookmarks' grid cell and the line. CSS and markup only. |
| T2 | §5: `containWithin` in `computeMenuPosition` and `FlyoutMenu`, used by both browser menus. |

One PR is fine; T1 alone is a smaller first step if T2 needs more trying.

## 8. Risks

- **The grid's columns change.** Four columns instead of three; a mistake shows as the address bar in the wrong cell. The unit test checks the cells and the live check covers both layouts.
- **A menu in a thin pane covers more of the page** than one that ran into the next pane did. That is the point (the page it covers is its own pane's), but it hides more of the page while open.
- **`autoUpdate` cost.** Measuring the pane's rect on each update is one more `getBoundingClientRect` per frame while a menu is open: negligible.

## 9. Open questions

1. **Where exactly Bookmarks goes in the thin layout.** This spec reads "on the right of the address bar, to the left of profiles" as the top row's right-hand group, next to Profile (§3.1). The other reading puts Bookmarks at the end of the address row, `[ address ] 🔖`, with Profile alone at the top right. Which one?
2. **The line's weight.** Same as the bar's bottom border (`--border-color`, 1px) per §4. A softer line (the border at reduced opacity) is an option if the full line looks heavy at chrome zoom 1.5, where it is 1.5 screen px.
3. **Mirror the Profile menu** (icons on the right) when it opens leftward? Default: no; decide when trying it.
