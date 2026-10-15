# SPEC: The browser pane's nav bar stacks in thin panes and follows chrome zoom

**Status:** active — built (B1), PR #4981. The live checks (B2, §6.2) are partly done: the operator tried the layouts and chrome zoom in a dev build on Windows on 2026-10-15; the measured checks in §4.2 are not yet recorded.
**Author:** Agent5@narko, 2026-10-14, at the operator's request
**Trigger (operator, 2026-10-14):** "lets now refine the browser chrome. It needs to be responsive (two line, address bar atop) for thinner panes. We also want its zoom bound to the overall agentmux chrome zoom. take a look, write spec to file"
**Changed after trying the build (operator, 2026-10-15):** "in the thin view, put the address bar below", then "we dont need the arrow .. <enter> will be the only option. Also, keep the profile on the right on both wide and thin". This spec describes the result; §10 lists what changed from the first draft.
**Builds on:** `zoom-architecture.md` (chrome zoom is CSS `zoom: var(--zoomfactor)` on a container), `docs/reports/REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md` (how portaled panels follow chrome zoom), `SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md` (the Go icon, which this removes) and `SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md` (the Profile button).

## 1. Summary

1. **Two layouts, one DOM.** A wide pane shows one row: `← → ↻ ⌂ 🔖 [address] 👤`. A thin pane shows two: the buttons on top, with Profile at the right end, and the address bar under them at full width (§3). The switch is a CSS container query on the bar. No element moves in the DOM, so a resize while typing never loses focus, the IME composition or the suggestion list.
2. **No Go button.** Enter is the only way to submit the address bar, as in Chrome.
3. **Chrome zoom.** The nav bar gets `zoom: var(--zoomfactor)`, the same rule the pane header, title bar and status bar use (§4). At chrome zoom 1.5 the bar, its buttons, the address text and the suggestion list are all 1.5× larger, matching the pane header right above it. Ctrl+wheel over the bar zooms chrome, as it does over the header.
4. **Page zoom is unchanged.** "Its zoom" means the browser *chrome*, the nav bar. The web page keeps its own per-pane zoom (Ctrl+wheel over the page, native, `REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20.md` §6 PR B). Chrome zoom never changes the page's zoom.
5. **Frontend only.** `browser-nav-bar.tsx`, `browser-view.scss`, and `isOverChrome()` moved from `app.tsx` to its own module so it can be tested (§5). No host, CEF or srv change.

## 2. What existed (verified on `main` @ `9cfe9c066`, 2026-10-14)

| Piece | State |
|---|---|
| The bar | `BrowserNavBar` (`frontend/app/view/browser/browser-nav-bar.tsx`), rendered first inside `.browser-view` (`browser-view.tsx:232`), only when `browser:show_controls` isn't false. One `div.browser-nav-bar`, a flex row: Back, Forward, Reload (`button.browser-nav-btn`), Home (`IconButton`), Bookmarks (a `FlyoutMenu` trigger), Profile (`BrowserProfileButton`), then `div.browser-address-wrap` (the input plus `AddressSuggestions`), then Go. |
| Sizes | `browser-view.scss`: bar padding 4px/8px, gap 4px, `border-bottom: 1px`; every button 28×28px; the input 28px high, `flex: 1; min-width: 0`, 12px mono. The bar is 37px high. |
| Thin panes | Nothing was responsive. The six buttons and Go took 240px including padding and gaps, and the address bar got what was left. At a 300px pane it was about 60px wide, which after its 20px padding showed about 5 characters of the URL. Below 240px it was 0px and the buttons overflowed the pane. |
| Chrome zoom | `--zoomfactor` on `:root`, written by `zoom.ts` with `chromeZoomAtom` (`store/chrome-zoom.ts`). Applied with `zoom: var(--zoomfactor)` to `.window-header` (per platform), `.status-bar`, and the pane header (`mixins.scss` `block-frame-default-header-layout`, line 35). The browser nav bar had no zoom, so at chrome zoom 1.5 a 50px pane header sat above a 37px nav bar with 12px text. |
| Ctrl+wheel | `isOverChrome()` (`app.tsx:221`) sends a wheel over `.window-header`, `.status-bar`, `.block-frame-default-header` or `[data-zoom-scope="chrome"]` to chrome zoom. Anything else in a pane goes to `zoomBlockIn/Out`, which does nothing for a browser pane (no `paneZoom` capability), so Ctrl+wheel over the nav bar was a no-op. Over the page it never reaches the DOM: the page is a native surface with its own zoom. |
| Native page geometry | `usePaneRectSync` sends the native page the rect of `.browser-placeholder`, which sits below the bar, from a `ResizeObserver` plus a safety-net interval. A taller bar makes the placeholder shorter, which the observer already catches. |
| Suggestion list | `AddressSuggestions` is an absolutely positioned child of `.browser-address-wrap` and registers itself as a pane overlay (`usePaneOverlay`) from its `getBoundingClientRect()`, so the native page is cut away under it. A suggestion row's `mousedown` is `preventDefault`ed, so clicking one doesn't blur the input. |
| Menus | Bookmarks and Profile open a `FlyoutMenu`, which portals to `document.body`, and Home's tooltip is portaled too. None of them follow chrome zoom anywhere in the app, including the title bar's own menus (`REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md` step 4, deferred). |
| Container queries | Used in the agent pane on a container that is itself zoomed (`agent-view.scss:396-414`, `@container agent-pane`). |

## 3. Responsive layout

### 3.1 The two layouts

```
Wide (bar ≥ 388px in its own px):
┌──────────────────────────────────────────────────────────────┐
│ ← → ↻ ⌂ 🔖 [ https://example.com/path                     ] 👤 │
└──────────────────────────────────────────────────────────────┘

Thin (bar < 388px):
┌──────────────────────────────┐
│ ← → ↻ ⌂ 🔖                 👤 │
│ [ https://example.com/path   ]│
└──────────────────────────────┘
```

- **Navigation group** (Back, Forward, Reload, Home, Bookmarks): on the left in both layouts.
- **Profile:** alone at the right end in both layouts, after the address bar in the wide one and across from the buttons in the thin one.
- **Address bar:** between them in the wide layout; its own row, at full width, under the buttons in the thin one.
- **Below about 208px** (the buttons plus padding), the navigation group wraps rather than overflowing the pane. This is a last resort; a pane that narrow is rare.

### 3.2 The breakpoint

The wide row needs 208px of fixed parts: five 28px buttons and four 4px gaps (156), the two 4px gaps around the address bar, Profile (28) and 16px padding. A 180px address bar shows about 22 characters of a 12px mono URL (about 7.2px each, after the field's 20px padding), enough to read the host and the start of the path. So the bar stacks below **388px**.

The 388px is measured in the bar's own CSS px, i.e. after chrome zoom: at zoom 1.5 the bar stacks below 582 screen px, because every part of it is 1.5× wider. Section 4.2 explains why the container query gives this for free.

It's named once, `$browser-nav-stack-below: 388px` in `browser-view.scss`, with the arithmetic in a comment. A future button in the bar adds 32px to it.

### 3.3 Markup

```tsx
<div class="browser-nav-bar" data-zoom-scope="chrome" onKeyDown={...}>    // zoom, container
  <div class="browser-nav-bar-grid">
    <div class="browser-nav-group browser-nav-group--nav">← → ↻ ⌂ 🔖</div>
    <div class="browser-address-wrap">input + AddressSuggestions</div>
    <div class="browser-nav-group browser-nav-group--profile">👤</div>
  </div>
</div>
```

The outer element is the query container (an element can't query its own size), and the grid inside it changes layout:

```scss
.browser-nav-bar {
    zoom: var(--zoomfactor);
    container: browser-nav / inline-size;
    // background, border-bottom and flex-shrink unchanged
}

.browser-nav-bar-grid {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    grid-template-areas: "nav address profile";
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
}

@container browser-nav (max-width: #{$browser-nav-stack-below - 0.02px}) {
    .browser-nav-bar-grid {
        grid-template-areas:
            "nav     .       profile"
            "address address address";
    }
}
```

Each child has its `grid-area`. The groups are `display: flex; gap: var(--space-1)`, with `flex-wrap: wrap` for §3.1's last resort.

### 3.4 Why one DOM and not two

- **Focus and typing survive a resize.** Dragging a splitter while the address bar has focus, mid-IME composition or with suggestions open, only changes the grid areas. Rendering two layouts, or moving the input with `<Show>`, would remount the input and drop all three.
- **Keyboard order is the same in both layouts:** Back → … → Bookmarks → address bar → Profile. It matches what you see in the wide layout. In the thin one Profile is drawn on the first row but reached last; keeping one order means a resize never changes what Tab does, and the address bar is also reached with Ctrl+L (`browser-pane-shortcut` → `focus-address`).

### 3.5 No Go button

Enter in the address bar submits it (`handleAddressKeyDown` → `handleNavigate`), as it always has, and is now the only way. Removing Go also removed its special case in the address bar's blur handler: Go used to keep a previewed suggestion when focus moved from the field to Go, because clicking Go submitted it. Now any blur drops a suggestion that was previewed but not opened. Clicking a suggestion is unaffected (its `mousedown` doesn't blur the field).

## 4. Chrome zoom

### 4.1 The rule

`zoom: var(--zoomfactor)` on `.browser-nav-bar`, as on the pane header. Everything in the bar is in px, so it all scales together: buttons, icons, the input's height, padding and 12px text, the gaps, and the suggestion list (a child of the bar, so its 360px `max-height` and 12px text scale too).

At zoom 1.0 the bar is 37px high in one row and 69px in two. At 1.5 those are 55.5px and 103.5px.

### 4.2 What follows from it, and what has to be checked

| Concern | Expected | Check in B2 |
|---|---|---|
| The breakpoint | The query container is the zoomed element, so its inline size in its own px is the pane width ÷ zoom, and 388px means "388 of the bar's own px". The agent pane relies on the same behaviour (`agent-view.scss:400-404`). | Find the screen width where the bar stacks, at zoom 0.65, 1.0 and 1.5 (≈252 / 388 / 582px). If the query sees the unzoomed width instead, fall back to §4.3. |
| The native page | The placeholder is below the bar, outside the zoom. When zoom or stacking changes the bar's height, the placeholder's height changes and `usePaneRectSync`'s observer resends the rect. | No gap and no overlap between the bar's bottom border and the page at every zoom, in both layouts, and right after a chrome zoom change. |
| The suggestion list's cut-out | `usePaneOverlay` reads `getBoundingClientRect()`, which in Chromium's standardized zoom (Chrome 128+, CEF here is 154) is in viewport px, already zoomed. So the cut-out matches the drawn list. | Open suggestions at zoom 1.5 over a page. On Windows no page shows through the list; on macOS/Linux the still frame covers exactly the list. |
| The suggestion list's stacking | The list (`z-index: 40`) must paint over the page placeholder after the bar. A stacking context on the bar would trap it under the placeholder. `container-type: inline-size` doesn't create one in Chrome 154 (it implies no layout containment; computed `contain` is `none`, and a reduced repro with `zoom: 1.5` showed the list on top), but `contain`, `opacity` or `transform` would. So the bar gets `position: relative; z-index: var(--z-pane-overlay)` (pane chrome over pane content, as the agent pane's tab strip) as a guard. Its menus and tooltips are portaled, and the new-profile dialog opens in the modal layer, so nothing else in the bar depends on this. | Open suggestions over a loaded page: the whole list shows, over the page area, at zoom 1.0 and 1.5. |
| The first-click select-all and caret | Unaffected: they use DOM selection, not coordinates. | Click into the bar at zoom 1.5: the whole URL is selected; a second click places the caret where clicked. |
| Menus and the Home tooltip | Portaled to `document.body`, so not zoomed. They are placed from the trigger's real rect, so they open in the right place, at 100% size. That matches the title bar's own menus. | The bookmarks and profile menus open under their buttons at zoom 0.65 and 1.5, and Profile's menu stays on screen at the bar's right end. |

### 4.3 Fallback if the container query doesn't see the zoomed width

Replace the query with a class set from a `ResizeObserver` on `.browser-nav-bar`: `stacked = bar.clientWidth < 388`. `clientWidth` is in the element's own px, so it already divides by zoom. The class only changes the bar's height, never its width, so it can't oscillate and needs no measurement mirror (unlike the top bar's tiers, `use-widget-bar-responsive.ts`).

### 4.4 Ctrl+wheel

`data-zoom-scope="chrome"` on `.browser-nav-bar`. `isOverChrome()` already honours it, so Ctrl+wheel over the bar zooms chrome, as over the pane header right above it. Ctrl+wheel over the page is untouched: it is native page zoom, per pane.

## 5. Changes

| File | Change |
|---|---|
| `frontend/app/view/browser/browser-nav-bar.tsx` | The bar's children in `div.browser-nav-bar-grid`: Back/Forward/Reload/Home/Bookmarks in `div.browser-nav-group--nav`, then the address wrap, then Profile in `div.browser-nav-group--profile`. `data-zoom-scope="chrome"` on the outer bar. The Go button, its ref and its blur special case removed (§3.5). Otherwise only indentation changes. |
| `frontend/app/view/browser/browser-view.scss` | `.browser-nav-bar`: `zoom`, `container`, and `position: relative; z-index: var(--z-pane-overlay)` (§4.2); the layout in `.browser-nav-bar-grid`; the group rules; the `@container` rule; `$browser-nav-stack-below` with its arithmetic; `grid-area` on the address wrap. |
| `frontend/app/store/chrome-zoom-target.ts` (new) | `isOverChrome()`, moved unchanged from `app.tsx` (which imports it), so it can be tested without `app.tsx`'s imports. |
| `frontend/app/view/browser/browser-nav-bar.test.tsx` (new) | See §6.1. |

## 6. Tests

### 6.1 Unit (vitest, jsdom)

jsdom doesn't do layout, container queries or `zoom`, so the unit tests pin the structure the CSS depends on:
- the bar carries `data-zoom-scope="chrome"`, and `isOverChrome()` is true for its address bar;
- the grid holds exactly the navigation group, the address wrap and the Profile group, in that order, with the input inside the address wrap, and there is no Go button;
- the navigation group holds Back, Forward, Reload, Home and Bookmarks, and the Profile group holds only Profile;
- `isOverChrome()` is true inside each element it accepts and false in pane content.

### 6.2 Live (own `task dev`, never the operator's instance)

In an isolated dev build, with screenshots for the PR:
1. Pane widths 260, 370, 410, 700 and 1200px at chrome zoom 1.0: wide or stacked as §3 says, no overflow, no gap between bar and page.
2. The §4.2 checks at zoom 0.65 and 1.5, including the breakpoint in screen px.
3. Type in the address bar with suggestions open, then drag the splitter across the breakpoint: focus, typed text and the suggestion list survive, and the list's cut-out follows it. Enter opens the highlighted suggestion.
4. Change chrome zoom with Ctrl+wheel over the nav bar: the bar and the pane header scale together, and the page doesn't change zoom.
5. With `browser:show_controls` false: no bar, page fills the pane as before.
6. A floating browser pane and a torn-off window: same layouts. The floater's edge-resize inset (`usePaneRectSync`) is unaffected.

Windows first (the operator's platform). macOS and Linux differ only in the overlay mechanism (§4.2 row 3); check them if a build is available, otherwise note it in the PR.

## 7. Out of scope

- **Menus following chrome zoom.** The bookmarks and profile `FlyoutMenu`s stay at 100%, like every `FlyoutMenu` in the app. Making `.menu` panels follow chrome zoom is `REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md` step 4, an app-wide change, not part of this.
- **Page zoom.** Binding the web page's zoom to chrome zoom isn't asked for and would fight the page's own per-pane zoom.
- **Hiding buttons in very thin panes** (an overflow "…" menu). The navigation group wraps below about 208px instead. Revisit only if that shows up in real use.
- **Button size changes.** The 28px buttons stay 28px; they only scale with zoom.

## 8. Phases

- **B1:** markup, CSS, unit tests (§5, §6.1). Built.
- **B2:** the live checks (§6.2), in the same PR before merge, with screenshots in the description. If the container query fails the breakpoint check, switch to §4.3 in that PR.

## 9. Open questions

1. **Breakpoint value.** 388px gives the address bar at least 180px in the wide layout. A higher value (e.g. 448px, a 240px field) stacks sooner and keeps more of the URL visible. Tune while trying the dev build.

## 10. Changes from the first draft (operator, 2026-10-15, after trying the build)

- The thin layout's address bar moved from the top row to the bottom row.
- The Go button was removed; Enter submits (§3.5).
- Profile moved to the right end in both layouts; Bookmarks joined the navigation group. The wide layout no longer looks exactly as before: Profile is now after the address bar.
- The breakpoint went from 420px to 388px, since the fixed parts went from 240px to 208px.
