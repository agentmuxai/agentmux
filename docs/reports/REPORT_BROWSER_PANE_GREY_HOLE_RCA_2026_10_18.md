# RCA: a solid grey rectangle over a browser pane (stale airspace hole from a background tab)

**Date:** 2026-10-18
**Build:** 0.59.32 (Windows)
**Status:** implemented. Fixes 1 and 2 below ship in the PR that adds this report; 3 and 4 are follow-ups.

## Symptom

A browser pane showed a large, flat dark-grey rectangle over the middle of its
page. The page under it was not drawn, and nothing was painted in its place.
A few minutes later it vanished without anyone touching the window.

## Evidence

The host's own input diagnostics, over about two minutes before the report:
- They recorded the pane as covered at its centre by the main window's render
  widget, the window that draws the app's DOM, not by any other pane or app.
- Scrolling over that spot went to the main window instead of the page.
- A browser pane in another window tab showed the same fault twice in the same
  half hour.
- The airspace clip the host applied never dropped to zero overlay rects while
  this lasted.

## What the rectangle was

A browser pane is a native child window drawn above the app's DOM ("airspace").
To let a menu, tooltip or panel in the DOM show over a pane, the frontend sends
the overlay's rectangle to the host, and the host cuts that rectangle out of
every pane window with `SetWindowRgn` (`crates/cef/src/browser_panes/clip.rs`).
Through the hole you see the main window's DOM.

So the grey rectangle was a hole in the pane's window region, with the overlay
that asked for it not painted. What showed through was the plain grey
background behind the pane. Input in that area went to the main window, which is
exactly what the host diagnostics recorded.

## Root cause

**An overlay inside a background tab kept its hole cut through the browser panes
of the tab on screen.**

1. **Overlay rects come from DOM elements, including ones in background
   tabs.** Inactive window tabs are never unmounted (`frontend/app/workspace/workspace.tsx`,
   the per-tab container around line 285). All tabs are stacked
   `position:absolute; inset:0` in the same area, so an element in a background
   tab has a real bounding rect at the same window coordinates as the visible tab.

2. **Hiding a tab does not tell any overlay that it is hidden.** The default
   (`window:keepinactivetabslaidout`, on unless set to `false`) hides an inactive
   tab with `visibility:hidden` and `opacity:0` on the tab container. The
   other mode uses `content-visibility:hidden`. Neither changes an overlay's own
   size or its own `style` attribute, which are the only things the
   overlay code watches:
   - `usePaneOverlay` (`frontend/app/platform/pane-overlay.ts`, `update()` at line ~401)
     re-measures on its element's ResizeObserver, a MutationObserver on its own
     `style` attribute, and `window` resize.
   - `pane-overlay-auto.ts` (`[data-pane-overlay]` elements) re-measures on the
     same observers, `transitionend`, and a sweep on window resize or scroll.

   A tab switch triggers none of these, so the rect registered while the tab was
   visible stays in the clip after the tab is hidden.

3. **Even a re-measure would not always drop it.**
   - `usePaneOverlay`'s `update()` checks only that the box is non-zero. It has no
     visibility check at all. Its comments assume an inactive tab is
     `display:none` and collapses to 0×0, so a ResizeObserver would drop the
     rect. That stopped being true when inactive tabs moved to
     `content-visibility:hidden` and then to `visibility:hidden` (kept laid out):
     both keep the real size.
   - `isOverlayElementVisible` (`pane-overlay-auto.ts:56`) and `registerPaneOverlay`
     (`pane-overlay.ts:328`) check computed `visibility`. That catches the default
     mode, because `visibility` inherits from the tab container, but only when
     something triggers a re-check. It never catches `content-visibility:hidden`,
     which leaves the descendants' computed `visibility` as `visible`.

4. **The clip is applied to every pane in the window.** `clip.rs` cuts the union
   of all overlay rects out of every live pane window, whichever tab the overlay
   belongs to. An overlay in tab B therefore cuts tab A's panes.

The hole lasted until the overlay unmounted or changed size or style, or the
window was resized. A few minutes later one of those happened, most likely the overlay
closing in its own tab (for example an agent's question or decision panel being
answered, or a menu or tooltip closing).

## What is not known

- **Which overlay it was.** By the time it was looked at, the hole was gone, and
  the overlay rects are not logged: `clip.rs` logs only how many there are. The
  candidates that stay open on their own in a background tab are the agent
  question and decision panels (`usePaneOverlay(p.getEl)`), an open
  menu or popover, a modal, a tooltip, and the pane drag placeholder. A browser
  pane's own loading cover in a hidden tab (a `data-pane-overlay` element) is
  a candidate too: it sits over its pane's rect, which in a hidden tab overlaps
  the panes of the tab on screen.
- No question or decision events were found in srv's log around that time,
  so that exact candidate is not confirmed.
- **Same mechanism, not seen in the incident:** a dormant pane-stack member (a
  kept-alive pane tab that isn't its stack's active one,
  `pane-leaf-chrome.tsx`) is hidden the same way as a laid-out window tab
  (`visibility: hidden` and `opacity: 0`, real size kept), so an overlay in one
  cuts the same stale hole. The fix covers it.

The host recorded no "covered" event afterwards, including over a later session
in which several browser panes in one tab were driven for about half an hour
with no tab switch. That is consistent with the cause: it needs an overlay in a hidden tab.

## Was anything changed while investigating?

No. The investigation only read logs and code, ran one layout query and one
window capture. There were no clicks, tab switches or setting changes. The hole
closed on its own.

## Fix

1. **Content of a hidden tab is not visible to the overlay code** (shipped).
   - `workspace.tsx` marks a window tab's container `data-window-tab-hidden`
     while it isn't the displayed tab. `pane-leaf-chrome.tsx` marks a dormant
     pane-stack slot `data-pane-dormant`.
   - `isInHiddenPaneTab(el)` (`pane-overlay.ts`) is `el.closest()` on those two
     markers. All three registration paths use it: `usePaneOverlay`,
     `registerPaneOverlay`, and `isOverlayElementVisible` for
     `data-pane-overlay` elements.
   - A marker works the same in both tab-hiding modes. Unlike a computed-style
     check, it can't be escaped by a descendant with an explicit
     `visibility: visible`, and it isn't fooled by `content-visibility: hidden`,
     which leaves descendants' computed visibility `visible`.
   - An overlay rendered through a Portal sits under `document.body`, outside
     its tab's DOM, so no marker is above it. `usePaneOverlay` reads
     `useWindowTabDisplayed()` instead: the window tab's displayed state that
     views already use to pause while hidden (`window-tab-visibility.ts`,
     `pane-tab-visibility.ts`).
   - The Portal-mounted `data-pane-overlay` elements mark themselves while
     their window tab isn't displayed (`useHiddenTabMarker()`): the tooltip,
     popover menu, anchored popover, flyout menu and submenu, and the agent
     pane's peek overlay. Overlays in the window's own chrome (the More
     dropdown, pinned-widget flyout, tab context panel) never sit in a tab and
     need nothing.
   - **Not covered:** the imperative context menu (`js-context-menu.ts`). It is
     plain DOM with no Solid context.
     - It closes on any click, since a backdrop covering the whole window
       catches it, including a click on another tab. It also closes on Escape.
     - It can still stay open across a tab switch made from the keyboard. That
       is the one case left; it would need the menu to close when the
       displayed tab changes.
2. **Every overlay is re-measured when a tab is hidden or shown** (shipped).
   One `MutationObserver` on the two marker attributes, for the whole document,
   calls `remeasurePaneOverlays()`. That runs every registered overlay's
   `update()` and the auto service's sweep. `usePaneOverlay` also re-measures
   when its displayed state changes. Per-overlay observers can't see this: an
   ancestor's visibility changes neither an overlay's size nor its own style.
3. **Scope holes to the overlay's own tab** (follow-up). Send each rect with the
   tab it belongs to, and cut it only from that tab's panes, so a future hiding
   mode missed by (1) and (2) can't leak a hole across tabs. This must not
   change how hidden browser panes themselves are sized or hidden: that is the
   memory work's planned `SetHidden` fix, agreed with its coordinator.
4. **Log the rects** (follow-up). Have `clip.rs` log the overlay rects, and the
   frontend the registrant's name, at debug level, so the next occurrence
   names the overlay.

**Tests** (`pane-overlay.test.ts`, `pane-overlay-auto.test.ts`,
`pane-overlay-hook.test.tsx`):
- An overlay under either marker registers no hole.
- Adding the marker drops the hole, and removing it restores it.
- `remeasurePaneOverlays()` re-reads a registered overlay.
- `isOverlayElementVisible` is false under either marker.
- `usePaneOverlay` cuts no hole while its window tab isn't displayed, cuts one
  when the tab is shown, and drops it when the tab is hidden again. Outside any
  window tab, it always clips.
- With the Windows auto service running, a Portal-mounted `data-pane-overlay`
  drops its hole when its window tab is hidden and gets it back when the tab is
  shown again.
- Every one of these tests fails with the check disabled.

## Side finding

Around the same tab switches, the pane drag snapshot was captured at the wrong
size and dropped, and some page screenshots timed out. This is not the cause of the
hole, but it is worth its own look.
