# Pane tab and document tab strips: the selected pane tab and the selected document tab are always fully shown

**Status:** implemented — #4986: the pane tab strip and the document tab strip reveal their selected tab on selection, pane focus, any click or input in the pane, mount or becoming visible, and a layout change, smoothly and clear of the edge fade; the window tab bar shares the moved helper unchanged. Checked live in a dev build on 2026-10-15 (22 pane tabs, 20 document tabs). The strips register by DOM rather than a Solid context (§4). Written 2026-10-15; the code reading below is checked against `main` at 3e104fa8b. Every design question was settled with the operator (§7); nothing is open (§8).
**Author:** Camper@narko, at the operator's request
**Scope:** [`frontend/app/element/PaneTabStrip.tsx`](../../frontend/app/element/PaneTabStrip.tsx) / `.scss` (the strip component, shared by pane tabs and document tabs), [`PaneHeaderTabStrip.tsx`](../../frontend/app/element/PaneHeaderTabStrip.tsx) (the pane header's strip), [`DocTabStrip.tsx`](../../frontend/app/doc-tabs/DocTabStrip.tsx) (the document tab strip), [`PaneChrome.tsx`](../../frontend/app/element/PaneChrome.tsx); the reveal helper, formerly `frontend/app/tab/tab-strip-scroll.ts` and used only by the window tab bar, is now [`frontend/app/element/strip-reveal.ts`](../../frontend/app/element/strip-reveal.ts), shared.
**Related:**
[`SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md`](SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md) (the pane tab strip's zoom and hidden-scrollbar overflow),
[`SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`](SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md) (pane tab widths, rename),
[`SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md`](SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md) §3 (the window tab bar already does this for window tabs).

## 0. Words used in this spec

AgentMux has three kinds of tab, and this spec never says "tab" alone (they match the [glossary](https://docs.agentmux.ai/glossary/)):

| Term | What it is | Where it shows | This spec |
|---|---|---|---|
| **window tab** | A whole layout of panes ("Tab 1", "Tab 2", …) | The tab bar along the top of the window (`tabbar.tsx`) | **Out of scope.** It already keeps its active window tab in view. |
| **pane tab** | One block stacked in a pane: an Agent and a Terminal sharing one pane are two pane tabs | The pane header's strip, "the pane tab strip" (`PaneHeaderTabStrip.tsx`) | **The subject.** |
| **document tab** | One file inside one block (the Editor's and the Media pane's files) | A strip under the pane header, "the document tab strip" (`DocTabStrip.tsx`) | **Also the subject** (operator, 2026-10-15). Same strip component as pane tabs. |

The **selected pane tab** is the one the pane is showing: the pane's active stack member (`activeId` on the pane tab strip). A pane has exactly one. The **selected document tab** is the file the Editor or Media block is showing (`activeId` on the document tab strip). A **strip** is a horizontally scrolling row that holds tabs of one kind and, last, the add (`+`) button.

A pane has a pane tab strip and, if its block is an Editor or Media view, also a document tab strip. **Each strip reveals its own selected tab, independently, by the same rules.** Where this spec says "the strip" it means either; where only one applies it says so.

## 1. The problem

A pane's pane tab strip holds one pane tab per stacked block. When there are more pane tabs than fit, the strip scrolls sideways (`overflow-x: auto`, scrollbar hidden; the mouse wheel over the strip scrolls it, `PaneTabStrip.tsx` ~495-525). Nothing keeps the **selected pane tab** in view:

- The user scrolls the strip so the selected pane tab is off-screen, or it is simply out of view when the pane opens.
- They click in the pane, or start typing in it (the agent pane's input box, say). The header still shows other pane tabs. Nothing says which pane tab they are in.

The document tab strip has the same problem: open many files in the Editor, scroll its strip, and the selected document tab is out of view while you type in the file.

The window tab bar solves the same problem for window tabs already: `tabbar.tsx` ~320 calls `revealTabInStrip` whenever the active window tab changes. The pane tab strip has no equivalent. (`PaneTabStrip.tsx` reacts to `activeId` only to play the landing cue for document tabs.)

## 2. What the user wants

When a pane's selected pane tab is not fully visible in its pane tab strip, or its selected document tab is not fully visible in its document tab strip, that strip scrolls so it is, at these moments:

1. **Selecting the tab or the pane.** A pane tab becomes the pane's selected pane tab, or a document tab becomes the block's selected document tab, by any route (a click on it, a keyboard command such as next document, a picker, a MuxAPI call, a file opened, a pane tab or document tab added or moved in), or the pane becomes the focused pane.
2. **Any click or input in the pane.** A click anywhere in the pane's content, or typing, pasting or any other input into it: the agent pane's input box, a terminal, the editor. The selected pane tab and the selected document tab belong to the pane you are working in, so both strips should show them as soon as you act there.

"Fully visible" means the whole tab (pane tab or document tab), including its close button, lies inside the strip's visible area **and clear of the 28px edge fade** (`scroll-edge-fade(28px)`, `PaneTabStrip.scss` ~142) on any side that still has more pane tabs beyond it. A pane tab half under a fade is not fully visible.

## 3. Triggers

A **reveal** is one scroll that brings the selected pane tab fully into view. It is requested, never continuous.

| Trigger | When | Animated |
|---|---|---|
| **Tab selected** | the strip's `activeId` changes (any source): a pane tab added, redocked or restored as the selected one; a document tab opened, or promoted when the document tab in front closes | yes (smooth), instant under reduced motion |
| **Pane focused** | the pane becomes the layout's focused pane by any route, including keyboard pane navigation and programmatic focus (`NodeModel.isFocused` goes true); both of the pane's strips reveal | yes (smooth) |
| **Interaction** | any click or input in the pane's content (§4), whether or not the pane was already focused; both of the pane's strips reveal | yes (smooth) |
| **Mount / became visible** | the strip mounts, or its pane goes from hidden to shown (`pane-tab-visibility.ts`) | no |
| **Layout changed under the selected pane tab** | the strip or its selected pane tab or document tab resizes (window resize, a neighbouring tab of the same kind closes, a rename widens the tab) while it is partly out of view *and the user has not scrolled the strip since the last reveal* | no |

**Never a trigger:** the user's own scroll of the strip (wheel, trackpad, scroll-by-drag), hover, a pane tab's or document tab's attention or activity flash, scrolling the pane's own content (reading a transcript), another pane's changes, and anything that starts inside a strip itself (a click on a pane tab or document tab in it already selects that tab, through *Tab selected*). A click in the document tab strip does not reveal the pane tab strip, and the reverse.

A reveal never fights the user's scroll of the strip: it runs once per trigger, and a wheel or drag on the strip during the animation cancels it. The user can scroll the strip away and it stays away until the next trigger. *Their next click or keystroke in the pane* is that trigger.

## 4. Interaction

One pane-scoped, capture-phase listener set on the pane's leaf, installed by `PaneChrome` and not per view, so it works the same for the agent input, a terminal, the editor and every other pane type. A strip registers itself with its pane when it mounts (under the pane's root element, `[data-role="pane"]`, in `pane-strip-registry.ts`; keyed by DOM rather than a Solid context, because a view's content is built before `PaneChrome` renders and is not under its owner, so a context would never reach a document tab strip inside it), and the listener asks every registered strip of that pane to reveal. It reacts to:

- **Clicks:** `pointerdown`, any button and touch, anywhere in the pane's content area.
- **Keys:** `keydown` for every key except a lone modifier (Shift, Ctrl, Alt, Meta, Caps Lock), so typing, Backspace, Enter, arrows and shortcuts all count.
- **Other input:** `beforeinput`/`input`, `paste`, and IME `compositionstart`, so input that doesn't arrive as a plain keydown (voice dictation, a paste from the menu, an emoji picker) counts too.

Rules:

- **Every qualifying event can request a reveal**, with no burst or pause rule: the user asked for it on any click or input. Requests are coalesced to one check per animation frame.
- **Cheap when there is nothing to do.** The handler reads no layout. It first checks the strip's overflow state from `createScrollEdges` (`moreLeft`/`moreRight`, already maintained): a strip with no overflow does nothing at all. Only inside the animation frame, and only when the strip overflows, does it measure the selected tab.
- **Already fully visible:** nothing happens, so ordinary typing in a pane whose selected pane tab is on screen has no effect on the strip.
- **Events inside a strip** are ignored by that strip (see "Never a trigger"); the pane's other strip still reveals on them.
- **Events from another pane** never reach this pane's listener; each pane has its own.

## 5. The reveal itself

One function, shared with the window tab bar: `revealTabInStrip(strip, tab, position)` is renamed to say what it does for any strip item (e.g. `revealInStrip`) and moved out of `frontend/app/tab/` to a shared location (e.g. `frontend/app/element/strip-reveal.ts`), keeping its tests. The window tab bar keeps calling it, with the behaviour it has today.

- **Writes `scrollLeft` (or `scrollTo` on the strip) only. Never `scrollIntoView`.** That also scrolls every scrollable ancestor and once shifted the whole app sideways (`tab-strip-scroll.ts` header; [`REPORT_LAN_UNDISCOVERABLE_AND_PAGE_SHIFT_2026_10_04.md`](../reports/REPORT_LAN_UNDISCOVERABLE_AND_PAGE_SHIFT_2026_10_04.md) §3).
- **Least scroll that makes the pane tab fully visible**, with the 28px fade reserved on each side that has more content, so the pane tab ends up beside the fade, not under it.
- **Last pane tab:** scroll to the end, so it and the `+` button after it show whole. **First pane tab:** scroll to 0.
- **Zoom:** the pane header is drawn with `zoom: var(--pane-tab-strip-zoom)`. Rects are in zoomed viewport pixels and `scrollLeft` is in the strip's own pixels, so divide by the measured zoom, as `revealTabInStrip` does today.
- **Timing:** one `requestAnimationFrame` after the trigger, so a just-added pane tab is mounted and laid out. While `animateWidth` is running on the strip (`WIDTH_TRANSITION_MS`), re-measure once when it ends, since pane tab widths change during it.
- **Motion: smooth** (operator, 2026-10-15): `scrollTo({ left, behavior: "smooth" })`, so a long jump reads as travel; an instant write when `shouldReduceMotion("transition")` (`frontend/app/motion/motion.ts`). The window tab bar's reveal is instant today and keeps its behaviour; the shared helper takes the behaviour as a parameter.
- **Hidden panes:** a strip inside a `content-visibility: hidden` panel reads and writes no geometry. It reveals once when it becomes visible.
- **One strip per pane:** each pane has its own strip and its own state; nothing is shared between panes.

## 6. Where it goes in the code

1. Move the helper and extend it: add the fade clearance and a `behavior` option (smooth for these strips). The window tab bar's call site changes only by the new name.
2. `PaneTabStrip.tsx` (both strips): add a `createEffect(on(() => props.activeId, …))` that requests a reveal of the selected tab's element (the strip already finds tab elements by id for the document-tab landing cue), plus a `ResizeObserver` entry for the layout-changed trigger. Register a `revealSelected(reason)` handle with the pane's registry (`pane-strip-registry.ts`). The reveal's timing and cancellation live in `strip-reveal-controller.ts`.
3. `PaneChrome.tsx`: request a reveal of every strip registered under its root on `isFocused` becoming true; install the capture-phase interaction listeners (§4) on the pane's leaf.
4. `DocTabStrip.tsx`: nothing beyond using the shared component, which registers itself. Its document tabs already animate on close and drag (`docDrag`); the reveal runs after that animation, like the width re-measure in §5.
5. The agent history view draws the same component as its own standalone row; it registers and reveals like any strip. Which kind of tab it shows is confirmed when this is built.

## 7. Resolved

Settled with the operator (2026-10-15): "typing" means typing into the pane, for example the agent pane's input box; and any sort of click or input in the pane counts (§4), with no burst rule. The kinds of tab in question are the **pane tab** and the **document tab** (§0).

Also settled on 2026-10-15, as the operator's call or by "use the best recommendations":

- **Document tabs are included**, alongside pane tabs.
- **Scrolling is smooth**, and instant under reduced motion (§5).
- **Scrolling the pane's own content** (reading a long transcript, scrolling a long file) does not trigger a reveal (§3). It would pull the strips around while the user is only reading.

## 8. Open questions

None.

## 9. Verification

- **Unit (pure):** the target-`scrollLeft` function for: already visible (no change), hidden left, hidden right, under the left fade, under the right fade, first tab, last tab, no overflow (no change), zoom 0.5/1/2.
- **Component:** with a mounted pane tab strip of ~20 pane tabs, and separately a document tab strip of ~20 document tabs, and a stubbed layout: each trigger in §3 reveals in each; the user's wheel is never fought (scroll away, then a hover, an attention flash and another pane's change leave it alone); a wheel during the animation cancels it; a click, a keydown, a paste and an IME composition in the pane each reveal; a lone modifier does not; events inside a strip do not reveal that strip but still reveal the pane's other one; events in other panes do not; the two strips of one pane reveal independently; a strip with no overflow reads no layout; a hidden strip reads no geometry; the reveal is smooth, and instant under reduced motion.
- **Regression:** the window tab bar's existing tests pass unchanged against the moved helper.
- **Live**, in an isolated `task dev` build over CDP (never the instance in use): a pane with ~20 pane tabs. (1) Scroll the strip away, click the pane body: the selected pane tab appears beside the fade. (2) Scroll away, then type in the agent pane's input: it appears on the first key. Scroll away again, click inside the pane: it appears again. (3) Select a far pane tab from a picker or a keyboard command. (4) Add a pane tab: it is revealed with the `+` after it. (5) Change the chrome zoom and repeat (1). (6) In an Editor pane with ~20 document tabs, repeat (1) and (2) for the document tab strip, open a file (its document tab is revealed), and close the document tab in front (the one that takes its place is revealed after the close animation). Screenshots saved with the PR.

## 10. Not in scope

- The window tab bar and window tabs (they already do this).
- A visible scrollbar, or any change to the wheel-to-horizontal-scroll behaviour.
- Reordering, pane tab widths, or an overflow menu for hidden pane tabs.
