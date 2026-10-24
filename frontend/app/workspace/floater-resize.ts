// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The invisible edge grab band on a floating pane, in CSS px.
 *
 * A floater is a frameless WS_POPUP; native edge-resize via the parent wndproc
 * never fires (the embedded CEF child consumes WM_NCHITTEST), so the resize is
 * driven from the DOM: `floating-pane-workspace.tsx` detects a pointerdown
 * within the band and drives `set_window_rect`. This applies on all three
 * platforms (see `posScale()` there for the per-platform coordinate spaces).
 *
 * The band is measured in CSS px, so it is already scaled for the display's DPI
 * (12 CSS px is 24 device px at 200%). Pane zoom (`term:zoom`) and chrome zoom
 * scale content and the header, not the window's viewport, so they do not
 * change it.
 *
 * Nothing is painted in the band; it is a hit-test zone, so a wider one costs
 * nothing visually, except on a floating *browser* pane: its web-content is a
 * separate OS child window layered over the floater's DOM, so it would cover
 * the band and swallow the pointerdown. `use-pane-rect-sync.ts` therefore
 * insets that child by `FLOATER_BROWSER_EDGE_INSET` on the left, right and
 * bottom (the top edge is over the header, already DOM), exposing the band
 * underneath. The inset shows the floater's own background as a matte around
 * the web content, and a wide matte reads as a border, so it stays smaller than
 * the band: on a browser pane the grabbable depth on those three sides is the
 * inset, and the band beyond it sits under the web content.
 *
 * History: 12px at the start (PR #1177), shrunk to 4px to hide the browser-pane
 * matte (PR #1829), restored to 8px (PR #2332), and widened again here to 12px
 * for every other pane, with larger corners, because 8px was still too thin to
 * find (Kanban #1641). The matte no longer sets the band's width, because the
 * browser pane has its own inset.
 *
 * SPEC_FLOATING_PANE_EDGE_RESIZE.
 */

/** Depth of the left, right and bottom edge bands. */
export const FLOATER_EDGE_RESIZE_BORDER = 12;

/** Depth of the top edge band. Smaller than the others because the top edge is over the pane
 *  header, whose buttons start a few px below it and keep their click targets. */
export const FLOATER_TOP_RESIZE_BORDER = 8;

/** How far a corner reaches along each edge from the corner: the bottom corners are
 *  this deep too, the top corners are `FLOATER_TOP_CORNER_DEPTH` deep. */
export const FLOATER_CORNER_RESIZE_SIZE = 20;

/** How far a top corner reaches down from the top edge, so the header buttons below it
 *  (close, maximize) keep most of their click targets. */
export const FLOATER_TOP_CORNER_DEPTH = 12;

/** How far a floating browser pane's web-content is inset from the left, right and bottom
 *  edges to expose the band (see above). Never more than the band. */
export const FLOATER_BROWSER_EDGE_INSET = 8;

/** Resize directions, in `WMSZ_*` order, as `floaterEdgeDirection` returns them. */
export const FLOATER_EDGE_NONE = 0;
export const FLOATER_EDGE_LEFT = 1;
export const FLOATER_EDGE_RIGHT = 2;
export const FLOATER_EDGE_TOP = 3;
export const FLOATER_EDGE_TOP_LEFT = 4;
export const FLOATER_EDGE_TOP_RIGHT = 5;
export const FLOATER_EDGE_BOTTOM = 6;
export const FLOATER_EDGE_BOTTOM_LEFT = 7;
export const FLOATER_EDGE_BOTTOM_RIGHT = 8;

/** The CSS cursor for each direction, indexed by the code above. */
export const FLOATER_RESIZE_CURSORS = [
    "",
    "ew-resize",
    "ew-resize",
    "ns-resize",
    "nwse-resize",
    "nesw-resize",
    "ns-resize",
    "nesw-resize",
    "nwse-resize",
] as const;

/**
 * Which resize band a point inside a `width` × `height` window is in, or
 * `FLOATER_EDGE_NONE`. Coordinates are CSS px from the window's top-left. A
 * corner wins over an edge, so a point near both reads as the corner.
 */
export function floaterEdgeDirection(x: number, y: number, width: number, height: number): number {
    const nearLeft = x <= FLOATER_EDGE_RESIZE_BORDER;
    const nearRight = x >= width - FLOATER_EDGE_RESIZE_BORDER;
    const nearTop = y <= FLOATER_TOP_RESIZE_BORDER;
    const nearBottom = y >= height - FLOATER_EDGE_RESIZE_BORDER;
    const cornerLeft = x <= FLOATER_CORNER_RESIZE_SIZE;
    const cornerRight = x >= width - FLOATER_CORNER_RESIZE_SIZE;
    const cornerTop = y <= FLOATER_TOP_CORNER_DEPTH;
    const cornerBottom = y >= height - FLOATER_CORNER_RESIZE_SIZE;

    if (cornerTop && cornerLeft) return FLOATER_EDGE_TOP_LEFT;
    if (cornerTop && cornerRight) return FLOATER_EDGE_TOP_RIGHT;
    if (cornerBottom && cornerLeft) return FLOATER_EDGE_BOTTOM_LEFT;
    if (cornerBottom && cornerRight) return FLOATER_EDGE_BOTTOM_RIGHT;
    if (nearLeft) return FLOATER_EDGE_LEFT;
    if (nearRight) return FLOATER_EDGE_RIGHT;
    if (nearTop) return FLOATER_EDGE_TOP;
    if (nearBottom) return FLOATER_EDGE_BOTTOM;
    return FLOATER_EDGE_NONE;
}

/** Controls that win over the band: a header button (Close, Maximize) sits within a few px of the
 *  window's edge, so a wider band would take a part of its click target. A press or hover on one is a
 *  click on the control, never a resize. The band still covers the gap around the control. */
export const FLOATER_BAND_CONTROL_SELECTOR = 'button, [role="button"]';

/** Whether `target` is, or is inside, a control that wins over the band. */
export function isFloaterBandControl(target: EventTarget | null): boolean {
    return target instanceof Element && target.closest(FLOATER_BAND_CONTROL_SELECTOR) !== null;
}
