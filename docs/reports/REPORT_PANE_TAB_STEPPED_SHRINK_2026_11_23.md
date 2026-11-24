# Report: Pane tabs that run out of space shrink in steps instead of scrolling

**Date:** 2026-11-23
**Status:** analysis. Investigation complete; it informed the build in PR #8192 (pane tabs shrink in steps).
**Scope:** `frontend/app/element/PaneTabStrip.tsx`, `PaneTabStrip.scss`, `pane-tab-pill.tsx`, `strip-reveal-controller.ts`, `frontend/app/block/block.scss` (the header's scroll range), `DocTabStrip.tsx`.
**Requirement:** when a pane's tabs do not fit, they should shrink in steps, as a browser's tab strip does: the text labels go first, then everything but the icon, with a highlight on hover. The horizontal scroll is what happens today; research whether the stepped shrink is better.

## 1. What the strip does today

- **Sizing.** A pill is as wide as its content: 8 px left padding, a 14 px icon, a 4 px gap, the label, the 16 px close button, 4 px right padding, a 1 px border. Its wrapper is `flex: 0 0 auto` with `max-width: 240px`: a pill never shrinks below its own width, and a label longer than the cap is ellipsised. There is no floor or ceiling beyond that. The one exception: a strip with a single tab may shrink to the strip (`:not(:has(.pane-tab-tip ~ .pane-tab-tip))`).
- **When they do not fit.** In a pane header the strip is `max-width: none` and does not scroll itself. The header has one scroll range (`.block-frame-header-scroll`) that holds the strip, its "+", the pane's inline header elements, the grip and the aux controls; the window controls after it stay pinned. The range scrolls (wheel, touch, no scrollbar) with a 20 px edge fade. A strip outside a header (the document tabs, the agent history strip) scrolls itself, with a 28 px fade.
- **What stays visible.** Everything, until a tab leaves the visible part of the range: icon, label, the close button's space (always laid out; its ✕ is shown on hover, on focus, while closing, and always on an unsaved ("attention") tab), the icon badge, the remote swatch, the progress line along the top (working, waiting, out of quota, signed out), the selected check mark, the colour underline of the active tab. A tab that scrolls out of view loses all of these with it: a working agent's progress line or an unsaved mark can be off-screen.
- **The selected tab stays in view** by the reveal (`strip-reveal-controller.ts`): on selection, pane focus, a click or input in the pane, mount or becoming visible, and a layout change, moving the least it takes, clear of the fade.
- **Shared by** the pane header (agent, terminal, other panes), the editor's document tabs (`DocTabStrip.tsx`) and the agent history strip. All use `PaneTabStrip`.
- **Zoom.** The inner row carries `zoom: var(--pane-tab-strip-zoom)`; the outer box is not zoomed. Widths measured in script differ by platform (WebKit and Chromium disagree on a zoomed element's `offsetWidth`), so thresholds must be stated in unzoomed px and divided by the zoom factor.

## 2. How other tab strips handle it

**Chrome**, from its public source (`tab_style.cc`, `tab_strip_layout.cc`, `tab.cc`; the constants are in DIP):

- Standard tab width 232. All tabs share the width equally and shrink together from 232 down to the active tab's minimum.
- The active tab's minimum is the close button plus the tab's insets (about 28 to 32; the exact figure depends on layout constants I did not resolve). Below that point the active tab holds that width and only the inactive tabs keep shrinking, down to a 16 DIP interior (about 26 including the overlap): icon only.
- Close button: always on the active tab, whatever its size. On inactive tabs it is hidden once the tab's content width is under 68 (100 on touch). The icon shows if it fits; an inactive tab with no room for any control shows the icon centred, clipped if need be.
- The title is shown only while a tab is wide enough; a pinned tab (content width 24) is icon-only and never has a close button, and is drawn as a normal tab again if it is 30 wider.
- Hover shows a card with the title and page; tab search is a separate control. From memory, not re-checked: a scrolling strip exists behind a flag, and by default there is no scroll, so with very many tabs they are slivers.

**Firefox** (from memory, not re-checked): tabs shrink to about 76 px, then the strip scrolls with arrow buttons; an "all tabs" menu lists them; the close button is hidden on inactive tabs under about 140 px.

**VS Code** (from memory): a setting chooses `fit` (a tab is as wide as its title, the default), `shrink` (tabs shrink to a minimum) or `fixed`; past the limit the tab row scrolls with a thin scrollbar; `wrapTabs` puts them on more rows.

**The trade-off.** Shrinking keeps every tab present and clickable but loses the label, which is the only thing that tells two agents' tabs apart when the icon is the same. Scrolling keeps labels but hides tabs and what they signal. Chrome's answer is to shrink, with a hover card for the title; the best of the others adds an all-tabs list as the fallback.

## 3. Recommendation for pane tabs

One decision per strip, not one per tab, so the tabs of a strip always look alike and a step never flickers when one label changes. The step comes from the room each tab gets: `share = room / tab count`, in unzoomed px, where `room` is what the header leaves the strip (§4). A pill's natural width is measured only to decide whether the strip fits at all.

| Step | Share per tab | What shows |
|---|---|---|
| 0 full | all natural widths fit | as today |
| 1 clipped | at least 90 px | icon, label ellipsised (tabs share the squeeze in proportion to their width, so the longest labels clip first), close on hover; none loses anything but characters |
| 2 no label on resting tabs | 48 to 90 px | the active tab keeps icon, a clipped label and close; others show the icon alone, with no close button (a click anywhere on the tab selects it; the selected tab keeps its close) |
| 3 icon only | 27 to 48 px | inactive tabs: icon only, a hover highlight and the tooltip that already carries the tab's title, seat, model and state; the active tab keeps icon and close (47 px) so it can be identified and closed |
| 4 fallback | below 27 px for inactive, or the active floor does not fit | keep the scroll of today (the header range), with the reveal; add an "all tabs" list only if the operator wants one (cost in §5) |

The thresholds are starting points to be set with the look checks. Use hysteresis of 8 px when a step changes so a strip sitting on a boundary does not flap, and move the step in a single frame (no animation of the width itself; a 120 ms crossfade of the label is enough).

**What must hold at every step**

- *The active tab is identifiable and closable.* It keeps its colour, underline and (steps 2 and 3) its close button; its floor is larger than the others'.
- *No status mark is lost.* The progress line, the icon badge, the remote swatch and the selected check mark are positioned on the pill, not in the label, so they survive. The one that does not is the unsaved mark: it is the close button, always shown, which does not fit an icon-only tab. In steps 2 and 3 an unsaved tab shows the badge dot on its icon (the existing `PaneTabBadge`) in the attention colour, and its close is reached by selecting the tab first.
- *Names for keyboard and screen readers do not change.* The pill's accessible name stays the label; hiding is visual (`clip`, not `display: none`), and the tooltip already exists. The close button keeps its `aria-label`.
- *Drag to reorder and drop targets.* Drop position uses the pill's own rect: at 27 px each half is 13 px, small but hit-able, and the 2 px insertion line is unchanged. A dragged ghost keeps the full label (drag data carries it). Double-click rename needs the label: at steps 2 and 3 the rename input takes the active tab's width and the strip stays at its step while it is open.
- *Chrome zoom.* Steps are chosen in unzoomed px against room divided by the zoom factor. Add look checks at 100, 125 and 150 %.

## 4. Where the room comes from

In a pane header the strip is one item of a scrolling range, so there is no width for the tabs to shrink against. The build gives the strip a width: `range width, less the natural widths of everything after the strip in the range (the "+", inline elements, grip, aux controls)`, never less than the step-4 floor. A `ResizeObserver` on the range and on those items feeds the decision, the way `top-bar-tier.ts` does for the top bar. Past the floor the range scrolls as it does today. The window's top bar already drops widget labels before its tabs shrink; the same measuring hook is the precedent.

## 5. What it costs

**Components.** `PaneTabStrip.tsx` (the step decision and a `data-step` on the strip), a new `pane-tab-steps.ts` (the pure decision, with hysteresis; the one thing that needs unit tests), `PaneTabStrip.scss` (the step rules and the hover highlight), `pane-tab-pill.tsx` (the visual-only hiding, the attention dot, the rename exception), `block.scss` and `blockframe.tsx` (the budget for the strip in the header range), `strip-reveal-controller.ts` (the reveal acts only at step 4), and `DocTabStrip.tsx` and the history strip (opt in, or follow the same decision from the start).

**Size.** One build PR of moderate size (about 400 changed lines with tests), plus a small follow-up if the all-tabs list is wanted (a menu of the tabs, with a count badge). The risk is the loop between the step and the widths it changes; deciding from share and not from content avoids it.

**Window tabs.** They have their own shrink (to about 60 px) and no icon-only state. They should follow the same rule later; the decision function would be shared. Not part of this build.

**Tests and checks.**
- Unit: the step decision, including the hysteresis and zoom division.
- Component: the classes and the names (accessible name unchanged, close button reachable, attention dot) at each step.
- Real browser: a layout test that measures pill widths and checks that no status mark is clipped at each step, at several strip widths and zooms, in the style of the Kanban card layout test.
- Look checks on macOS and Linux (any visual change): each step at 100, 125 and 150 % zoom, dark and light, with a working, a waiting and an unsaved tab among them; the hover highlight and tooltip; a drag reorder at step 3; a narrow pane (about 300 px) where step 4 starts.

## 6. Decisions for the operator

1. Steps as in §3, with the active tab's floor larger than the others' (recommended), or equal widths as Chrome does.
2. After the last step: keep the scroll (recommended, no new control), or add an all-tabs list.
3. Document tabs in the editor follow at the same time (recommended, they share the strip), or pane header tabs only first.
