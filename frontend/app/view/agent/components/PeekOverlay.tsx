// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekOverlay — Portal-rendered hover-to-peek panel, anchored to the TOP
 * edge of the hovered entry (per 2026-08-03 user feedback: "when it appears
 * we need it to appear at the top of the entry") and flush to its left/
 * right edges, but rendered OUTSIDE the virtualized transcript's per-row
 * DOM subtree.
 *
 * Why not a plain `position: absolute` child of the row? Each row in the virtualized document
 * (`.agent-document-row`) carries `contain: layout` and a `transform`
 * (identity matrix, but any non-`none` transform still counts) — both
 * independently force a NEW STACKING CONTEXT per CSS spec. A `z-index`
 * inside one row's stacking context can never out-rank a LATER SIBLING
 * row's entire subtree, no matter how high the number: sibling stacking
 * contexts stack strictly by DOM order. Confirmed live via CDP:
 * `elementFromPoint` at the overlay's own screen coordinates returned a
 * LATER row's own content, not the overlay itself.
 *
 * Portal-rendering escapes every row's stacking context (renders at
 * `document.body`, in the root stacking context), so this always paints
 * above the whole transcript regardless of virtualization internals — the
 * same reason the original floating-ui `Tooltip` component never had this
 * bug. Position is `fixed` (not `absolute`) using raw
 * `getBoundingClientRect()` viewport coordinates, and floating-ui's
 * `autoUpdate` keeps it synced on scroll/resize without hand-rolling
 * scroll listeners.
 *
 * **Pane zoom (2026-09-06).** Escaping the row's subtree also escapes the
 * pane's `zoom`, which `agent-view.tsx` sets on the pane root — so this
 * panel used to render at 100% no matter how far the agent pane was zoomed
 * in or out, while everything around it scaled. An earlier revision of this
 * header called the component "already zoom-safe", which was true only of
 * its POSITION: `getBoundingClientRect()` reports post-zoom viewport pixels,
 * so the panel always landed in the right place — at the wrong size. That
 * half-truth is precisely why the gap survived. It now reads the pane's
 * factor off the anchor row's inherited `--agent-pane-zoom` and applies it
 * to itself; see `readPaneZoom` and `withPaneZoom` for the mechanism and
 * for why every length has to be divided by the factor.
 *
 * Positioning ("end" mode) is peek-placement.ts's job, and the panel always sits
 * near the pointer. Horizontally: right-aligned to the row when it fits the row's
 * width; when wider, reaching over the pane borders by at most half the row's width,
 * split by the pane's place in the window (all to the right at the left edge, all to
 * the left at the right edge, even when centred). Vertically: below or above the pointer
 * inside the transcript when it fits; otherwise it leaves the transcript, on the
 * side of the pointer with more room, with its HEIGHT cut to that room so it can
 * never reach the pointer, and the rest scrolls. The old code clamped the
 * position into the container and let the panel grow back over the pointer,
 * which flickers (the row sees `mouseleave`, the panel closes, the row sees
 * `mouseenter`, it reopens).
 *
 * A panel that had to be cut scrolls, so it is meant to be ENTERED, and four
 * things make sure the pointer can get there:
 *  1. it is PINNED once it turns out to scroll: it stops following the pointer
 *     (a panel that follows moves away from a pointer heading for it, and its
 *     height, cut to the room left, shrinks as it does);
 *  2. its near edge is FLUSH with the row (`flushToRow`), so the way to it does
 *     not cross a strip of the next row;
 *  3. it lingers after the row's `mouseleave`, and the linger is re-armed while
 *     the pointer keeps closing in on it (HOVER_BRIDGE_MS, up to
 *     HOVER_BRIDGE_MAX_MS), then stays while the pointer is on it;
 *  4. while the pointer is crossing to it, other rows' peeks wait
 *     (peek-bridge.ts), so one it passes over on the way cannot open on top.
 * A panel that fits behaves as before: it follows the pointer and closes the
 * instant the pointer leaves the row.
 * docs/reports/REPORT_TOOL_HOVER_PANEL_SIZE_AND_PLACEMENT_2026_10_02.md
 *
 * It additionally tracks the mouse's Y position while
 * hovering (2026-09-03, SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md):
 * horizontal position stays pinned to the row's right edge exactly as
 * before, but `top` follows the cursor (offset by CURSOR_GAP_PX below it —
 * see `update()`'s own comment for why the offset must be nonzero) instead
 * of freezing at the row's top edge, clamped to the scroll container's own
 * bounds.
 *
 * The panel shrink-wraps its content and pins its RIGHT edge to the row's right edge, growing
 * leftward: the metadata peek is two short lines (timestamp, token estimate) that shouldn't
 * stretch a full pane's width, and floating them right keeps them off the text you're actually
 * reading on the left. Right-alignment is done with `left: rect.right` + a `translateX(-100%)`
 * rather than a `right:` offset, deliberately: `right` would need `window.innerWidth`, a
 * different coordinate space from the `getBoundingClientRect()` values everything else here
 * uses. It also keeps the pane-zoom compensation in `withPaneZoom` uniform: every length it
 * touches is a `getBoundingClientRect()`-derived viewport pixel, divided by the same factor. A
 * `right:` offset would need the opposite correction.
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { createPeekBridge, usePeekLinger } from "./peek-overlay-bridge";
import { createPeekMeasure } from "./peek-overlay-measure";
import { createPeekPlacement } from "./peek-overlay-placement";
import { createPeekState, type PeekOverlayProps } from "./peek-overlay-state";
import { usePeekTracking } from "./peek-overlay-tracking";

/**
 * The overlay is a state object (peek-overlay-state.ts) and a run of hook groups that act on it, called in
 * the order their computations and effects must be created: the linger and marker, then the tracking.
 */
export function PeekOverlay(props: PeekOverlayProps): JSX.Element {
    const s = createPeekState(props);
    const bridge = createPeekBridge(s);
    usePeekLinger(s, bridge);
    const placement = createPeekPlacement(s, createPeekMeasure(s));
    const { registerFloating } = usePeekTracking(s, placement, bridge);

    return (
        <Show when={s.open()}>
            <Portal>
                <div
                    ref={registerFloating}
                    class={clsx("agent-node-peek-overlay", props.class)}
                    data-pane-overlay={s.outside() ? "" : undefined}
                    data-window-tab-hidden={s.hiddenTab()}
                    onMouseEnter={() => {
                        bridge.clearBridgeTimer();
                        s.held = true;
                        // Arrived: the crossing is over. (Rows under the panel
                        // cannot be hovered while the pointer is on it anyway, so
                        // no bridge is needed here, and none can go stale if a
                        // mouseleave is ever missed.)
                        bridge.releaseBridge();
                    }}
                    onMouseLeave={() => {
                        s.held = false;
                        // Pointer left the panel: linger briefly (it may be heading
                        // back to the row, whose mouseenter will re-show it).
                        if (!s.show()) bridge.startLinger();
                    }}
                    style={s.floatingStyle()}
                >
                    {props.children}
                </div>
            </Portal>
        </Show>
    );
}

PeekOverlay.displayName = "PeekOverlay";
