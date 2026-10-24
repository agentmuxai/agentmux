# Spec: The drop target lands inverted — a pane tab dragged over another pane's tab bar flashes once and then stays in the negative until the drag ends

**Status:** active. Shipped in #5270 (the change and its test). Left: the operator's check in `task dev` (§7), and the decision whether the Window Tab bar and the document tabs get the same treatment (§6).
**Date:** 2026-10-21
**Author:** Lazo
**Verified against:** `agentmux` `main` @ `afd0387db` (2026-10-21), by reading the stylesheets and the code that applies the class.
**Builds on:** [SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md](SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md) (§3, the flash this changes), [SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md](SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md) (§3.4, the drop-to-append hover).

## 1. Ask

The operator, 2026-10-21: "when dragging pane tabs between pane tab bars, we see a flash on the landing zone, but it should end in the off color until the user lets go. Currently it flashes but lands on the normal color."

## 2. What happens today

Dragging a tab from one pane over a different pane's tab bar marks that pane's header row as the drop zone (`.pane-header--foreign-hover`, applied by `foreignDropRootFor` in `PaneTabStrip.tsx`; a strip that is its own drop zone gets `.pane-tab-strip--foreign-hover`). Both use the `drop-target-hover` mixin of `drop-feedback.scss`:

```scss
animation:
    tab-drop-invert-strobe 400ms steps(1, end) 2,      // two blinks over 800 ms
    tab-drop-pulse 0.8s var(--ease-in-out) infinite alternate;
```

and `tab-drop-invert-strobe` is `0% invert(1) → 50% invert(0) → 100% invert(0)`: negative for 200 ms, normal for 200 ms, twice, and then it is over. The animation has no fill mode, so when it ends the row goes back to its normal colours, and the only thing left for the rest of the hover is the pulsing outline. That is the "flash, then lands on the normal colour" in the report.

## 3. What it should do

**Flash once, then land in the negative and stay there until the pointer leaves or the tab is dropped.** The landing zone is then unmistakable for the whole drag, not only for the first 800 ms.

This reads "the off color" as the inverted (negative) colour: the one colour the flash alternates with. If the operator meant another colour, the change is the final keyframe of `tab-drop-invert-hold` (§4) and nothing else.

- **Entry:** the row is negative for 200 ms, normal for 200 ms, then negative again, and stays negative. One blink, and two inversions in the first 400 ms.
- **Leave or drop:** the class is removed, so the animation and its held state end at once and the row is normal.
- **Re-entry:** a new hover entry replays the flash from the start, as today (the class is re-applied per entry).
- **The outline pulse** carries on over the held negative, unchanged.
- **The tabs themselves stay un-inverted**, as today: the header's negative is cancelled on the pill strip by running the same animation on it (a double invert is the original pixels). The cancelling has to hold too, so the strip also lands inverted.

## 4. Design

- `drag/drop-indicators.scss`: a new keyframes `tab-drop-invert-hold`: `0% invert(1)`, `50% invert(0)`, `100% invert(1)`. With `steps(1, end)` between keyframes it is negative for 0 to 200 ms, normal for 200 to 400 ms, and, with `animation-fill-mode: forwards`, negative from 400 ms on.
- `drop-feedback.scss`: `drop-target-hover($hold: false)` and `drop-target-strobe-exempt($hold: false)` get a `$hold` form that uses that animation with `forwards`. The default is unchanged, so no other user of the mixins changes.
- `element/PaneTabStrip.scss`: `.pane-tab-strip--foreign-hover`, `.pane-header--foreign-hover` and its pill-strip wrapper use `$hold: true`.
- **Reduced motion:** the reduced treatment (`feedback`) drops the blink, as it did the strobe. With `$hold` it holds the negative with no animation (`filter: invert(1)`), and the cancelling wrapper does the same, so the end state is the same and nothing flashes.
- **WCAG 2.3.1** (no more than three flashes in any one second): a single blink is two inversions in the first 400 ms and then a steady state. The earlier two-blink design had three transitions in 800 ms; this has fewer.

> **Update 2026-10-24:** the blink is now three 200 ms cycles (not one 400 ms cycle), the pulse 0.4 s a direction, and a window-wide blink budget keeps WCAG 2.3.1's three flashes a second across re-entries. See SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §3.

## 5. Why only the Pane Tab strip

The report is about pane tab bars. The mixin is shared with the Window Tab bar (`tab/tabbar.scss`) and the document tabs (`doc-tabs/doc-tabs.scss`), which keep the flash that settles back to normal. A shared look was the point of the 2026-09-24 spec (§3), so this is a deliberate difference to review, not an accident: §6.

## 6. Open

- **Window Tab bar and document tabs.** Should they land inverted too? It is a one-word change per file (`($hold: true)`), and the tests pin today's behaviour so the change is visible. Left alone until the operator says.
- **Whether "the off colour" is the negative.** See §3.

## 7. Tests and checks

- `frontend/app/drop-feedback.test.ts` compiles the real stylesheets (jsdom runs no CSS animations) and checks: the hold keyframes are negative, normal, negative; the pane header and the strip's own drop zone use the hold animation with `forwards`, keep the pulse, and no longer use the settle-back strobe; the pill-strip wrapper runs the same animation; the reduced-motion treatment holds the negative with no blink; the Window Tab bar still uses the settle-back strobe and not the hold.
- **By eye in `task dev`** (not verifiable in a headless run): drag a tab from one pane onto another pane's tab bar. The target row blinks once, then stays negative with the pulsing outline while the pointer stays over it; the tabs in it are not inverted; moving off returns it to normal at once; dropping it adds the tab and shows the normal row; re-entering blinks again.
