// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Size of the drag "ghost" — the native drag image you hold while dragging a
 * pane out of the tiled layout.
 *
 * Shared by all three platform TileLayout implementations
 * (`TileLayout.{win32,linux,darwin}.tsx`), which previously each hard-coded an
 * identical `DragPreviewWidth/Height = 300`. One definition rather than three
 * copies that agree today and drift later.
 *
 * **Why not the pane's literal size.** The obvious reading of "make the ghost
 * match the pane" is to use its full rect, but a ghost at true pane size
 * occludes the drop targets it is being aimed at — drag a half-screen pane and
 * you are holding a half-screen image over the split indicators and tab strip
 * you are trying to hit. It makes the drag harder, not more informative, and
 * `toPng` cost scales with area on a path that runs on every header hover.
 *
 * So: the pane's **aspect ratio**, scaled to fit a bounding box. A wide
 * terminal reads as wide, a tall pane reads as tall, and it stays out of the
 * way. The fixed 300×300 square this replaces was the thing that actually
 * looked wrong — every pane was held as a square regardless of its shape.
 */

/** Longest edge of the ghost, in CSS px. */
export const DRAG_PREVIEW_MAX_PX = 360;

/**
 * Shortest edge floor, in CSS px. An extreme aspect ratio (a very wide, short
 * pane) would otherwise scale to a sliver a few px tall that reads as a line
 * rather than a pane.
 *
 * Bounded by the pane's own dimension, so it can only ever undo part of a
 * shrink — it never enlarges an axis past its real size.
 */
export const DRAG_PREVIEW_MIN_PX = 96;

/**
 * Used when the source element can't be measured (not yet mounted, or a
 * degenerate 0×0 rect). Square, matching the previous fixed behaviour — the
 * honest answer when the real shape is unknown.
 */
export const DRAG_PREVIEW_FALLBACK: DragPreviewSize = { width: 300, height: 300 };

export type DragPreviewSize = { width: number; height: number };

/**
 * Scale `rect` down to fit within `DRAG_PREVIEW_MAX_PX` on its longest edge,
 * preserving aspect ratio.
 *
 * Never scales UP, on **either** axis: a pane already smaller than the box is
 * shown at its own size, since enlarging it would misrepresent what is being
 * dragged and blur the rasterised image. That applies to the short-edge floor
 * too — see the `Math.min(w, …)` clamp below.
 */
export function computeDragPreviewSize(rect: DragPreviewSize | null | undefined): DragPreviewSize {
    if (!rect || !Number.isFinite(rect.width) || !Number.isFinite(rect.height)) {
        return DRAG_PREVIEW_FALLBACK;
    }
    const w = Math.round(rect.width);
    const h = Math.round(rect.height);
    if (w <= 0 || h <= 0) {
        return DRAG_PREVIEW_FALLBACK;
    }

    // `Math.min(1, …)` is the never-scale-up rule.
    const scale = Math.min(1, DRAG_PREVIEW_MAX_PX / Math.max(w, h));
    const scaled = { width: Math.round(w * scale), height: Math.round(h * scale) };

    // Raise a collapsed edge back to the floor, but never past the pane's own
    // size. Both clamps are load-bearing:
    //
    //   `Math.max(…, floor)` rescues a genuine downscale sliver — a 4000x128
    //   pane scales to 360x12, which reads as a line rather than a pane, so
    //   the short edge is lifted to 96 and aspect ratio is deliberately broken.
    //
    //   `Math.min(w, …)` keeps the never-scale-up promise. Without it a 50x30
    //   pane (already inside the box, so scale === 1) came back as 96x96 —
    //   bigger than the thing being dragged, contradicting this function's own
    //   docstring. It also removes a discontinuity at the cap: 360x40 and
    //   361x40 now both yield 360x40 rather than jumping to 360x96.
    //
    // The floor is itself capped by MAX so the two constants cannot invert.
    const floor = Math.min(DRAG_PREVIEW_MIN_PX, DRAG_PREVIEW_MAX_PX);
    return {
        width: Math.min(w, Math.max(scaled.width, floor)),
        height: Math.min(h, Math.max(scaled.height, floor)),
    };
}

/**
 * Cursor grab-point offsets for `nativeSetDragImage`.
 *
 * MUST be computed from the size the image was actually rasterised at, not
 * from a nominal constant — if the two disagree the ghost visibly detaches
 * from the cursor. That is why `TileLayout.*` stores the size alongside the
 * cached image rather than recomputing it at drag-start.
 *
 * Preserves the original `+ 10` nudge so the ghost sits slightly below-right
 * of the cursor instead of directly under it.
 */
export function dragPreviewCursorOffset(size: DragPreviewSize, dpr: number): { x: number; y: number } {
    const ratio = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
    return {
        x: (size.width * ratio - size.width) / 2 + 10,
        y: (size.height * ratio - size.height) / 2 + 10,
    };
}
