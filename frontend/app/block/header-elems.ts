// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Splitting a pane header's `HeaderElem[]` into what must stay on the row
 * and what may move into the hover tooltip.
 *
 * SPEC_PANE_HEADER_TEXT_HOVER_TOOLTIP_2026_09_21.md moved the header's
 * informational text off the row (the Pane Tabs redesign left it a thin
 * sliver of width) and into a hover tooltip. `viewText()` is not uniformly
 * informational, though — the same array also carries real controls, and
 * the first cut moved those too:
 *
 *   - termViewModel's "Multi Input ON" textbutton (`click to disable`)
 *     became unreachable. The tooltip is anchored below the header with a
 *     6px offset and closes on the header's own `onMouseLeave`, so a
 *     pointer travelling toward it leaves the header — and dismisses the
 *     button — before arriving. Reported as P1 on PR #3488.
 *   - It also hid a persistent mode warning ("input is going to every
 *     connected terminal") behind a hover, which is exactly the kind of
 *     thing that has to stay visible.
 *
 * Hover-bridging the tooltip would have fixed the first point and not the
 * second. Partitioning fixes both: controls keep rendering inline exactly
 * as they always did, and only passive content moves. It is also stable
 * against future `viewText()` producers — a new control is inline by
 * construction, without anyone having to remember this trap.
 *
 * Kept in its own module (no imports) so it stays directly unit-testable —
 * importing `blockframe.tsx` pulls in the store, RPC, and layout graph.
 */

/**
 * Whether this element is a control the user can act on, rather than
 * something to read.
 *
 * "Can act on" is deliberately strict: an element that merely *could*
 * carry a handler but doesn't (a `textbutton` with no `onClick`, a
 * `noAction` or `disabled` iconbutton) is passive and belongs in the
 * tooltip with the rest of the status content. Being wrong in this
 * direction only costs a little header width; being wrong in the other
 * direction makes a control unclickable, which is the bug this exists to
 * prevent.
 */
export function isInteractiveHeaderElem(elem: HeaderElem): boolean {
    switch (elem.elemtype) {
        // Always a control — no variant of these is purely decorative.
        case "input":
        case "toggleiconbutton":
        case "connectionbutton":
        case "menubutton":
            return true;
        case "iconbutton":
            return !!elem.click && !elem.noAction && !elem.disabled;
        case "textbutton":
            return !!elem.onClick;
        case "text":
            return !!elem.onClick;
        case "div":
            // A div is a grouping element: it's interactive if it handles
            // anything itself, or if anything it contains does. Its own
            // hover handlers count — a div using onMouseOver/onMouseOut to
            // reveal detail can't do that from inside a tooltip that is
            // itself hover-gated.
            return (
                !!elem.onClick ||
                !!elem.onMouseOver ||
                !!elem.onMouseOut ||
                (elem.children ?? []).some(isInteractiveHeaderElem)
            );
        default:
            // Unknown future elemtype: treat as passive. A new *control*
            // should add its case above; defaulting the other way would
            // silently pin every new informational element to the row and
            // re-create the width problem the spec fixed.
            return false;
    }
}

/**
 * Partition header elements into `inline` (rendered on the header row, as
 * before) and `tooltip` (shown only while hovering the header).
 *
 * Relative order is preserved within each list. A `div` is never split
 * across the two — its children are laid out by the div itself, so
 * scattering them across two render sites would break whatever grouping
 * it exists to express; one interactive descendant pins the whole div
 * inline.
 */
export function partitionHeaderElems(elems: HeaderElem[]): { inline: HeaderElem[]; tooltip: HeaderElem[] } {
    const inline: HeaderElem[] = [];
    const tooltip: HeaderElem[] = [];
    for (const elem of elems) {
        (isInteractiveHeaderElem(elem) ? inline : tooltip).push(elem);
    }
    return { inline, tooltip };
}
