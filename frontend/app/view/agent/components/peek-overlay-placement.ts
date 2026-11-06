// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Placing the panel: where it sits, how wide and tall it may be, and whether it is pinned. */

import { findScrollContainerRect } from "./hover-anchor";
import { computePeekHorizontal, computePeekVertical } from "./peek-placement";
import type { PeekMeasure } from "./peek-overlay-measure";
import type { PeekState } from "./peek-overlay-state";
import { PEEK_MIN_WIDTH_ABOVE_HEIGHT_PX, PEEK_MIN_WIDTH_PX, readPaneZoom, withPaneZoom } from "./peek-zoom";

export function createPeekPlacement(s: PeekState, m: PeekMeasure) {
    const update = () => {
        const row = s.props.rowEl();
        if (!row) return;
        const rect = row.getBoundingClientRect();
        const container = findScrollContainerRect(row);
        const paneZoom = readPaneZoom(row);
        // Shrink-wrapped, near the pointer. Horizontal and vertical are decided
        // by peek-placement.ts (see its header): right-aligned to the row, or
        // reaching over the pane borders, split by the pane's place in the
        // window, when wider than the row; below/above the pointer, leaving the
        // transcript and cutting its
        // height to the room when too tall. The invariant it keeps — the
        // pointer is never inside the panel — is swept by its tests.
        const viewport = { width: window.innerWidth, height: window.innerHeight };
        const rowRect = { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
        // Placed first without a minimum, to see the panel's natural shape. Only
        // if that is more than two lines tall does the minimum width apply
        // (PEEK_MIN_WIDTH_ABOVE_HEIGHT_PX), and the panel is re-placed with it.
        // Both measurements are at fixed inputs, so the outcome never depends on
        // the previous placement. The px constants are in the panel's own pixels,
        // so they scale with the pane's zoom; placement works in viewport px.
        const naturalWidth = m.measureWidth();
        const readableWidth = m.measureReadableWidth();
        let hz = computePeekHorizontal({ row: rowRect, viewport, naturalWidth, readableWidth });
        let naturalHeight = m.measureHeightAt(hz.maxWidth, 0, paneZoom);
        if (naturalHeight > PEEK_MIN_WIDTH_ABOVE_HEIGHT_PX * paneZoom) {
            hz = computePeekHorizontal({ row: rowRect, viewport, naturalWidth, readableWidth, minWidth: PEEK_MIN_WIDTH_PX * paneZoom });
            naturalHeight = m.measureHeightAt(hz.maxWidth, hz.minWidth, paneZoom);
        }
        const placeAt = { row: rowRect, container, viewport, naturalHeight };
        let vt: ReturnType<typeof computePeekVertical>;
        if (s.pinnedOffsetY != null) {
            vt = computePeekVertical({ ...placeAt, mouseY: rect.top + s.pinnedOffsetY, flushToRow: true });
        } else {
            vt = computePeekVertical({ ...placeAt, mouseY: s.lastMouseY });
            // Enterable: pin it, and keep its near edge on the row so the way to
            // it does not cross another row. Decided on the unpinned placement
            // and then sticky until it closes, so the two never alternate.
            if (vt.scrolls && s.lastMouseY != null) {
                s.pinnedOffsetY = s.lastMouseY - rect.top;
                vt = computePeekVertical({ ...placeAt, mouseY: s.lastMouseY, flushToRow: true });
            }
        }
        // Enterable if pinned, or if it scrolls with no pointer Y to pin to yet
        // (a peek opened by the drag-release hover resync before any mousemove:
        // it still needs its linger, and pins on the first move).
        s.lastEnterable = s.pinnedOffsetY != null || vt.scrolls;
        s.setOutside(hz.extendsPastRow || vt.leavesContainer);
        s.setFloatingStyle(
            withPaneZoom(
                {
                    position: "fixed",
                    left: `${hz.left}px`,
                    top: `${vt.top}px`,
                    ...(hz.shift > 0 ? { transform: `translateX(${-hz.shift * 100}%)` } : {}),
                    "max-width": `${hz.maxWidth}px`,
                    "min-width": `${hz.minWidth}px`,
                    "max-height": `${vt.maxHeight}px`,
                },
                paneZoom,
            ),
        );
    };

    return { update };
}

export type PeekPlacement = ReturnType<typeof createPeekPlacement>;
