// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Atlas's pan and zoom: the map's position and scale in the pane, as the Drone canvas keeps its own (drone-view.tsx). */
export interface Viewport {
    x: number;
    y: number;
    k: number;
}

export const MIN_ZOOM = 0.15;
export const MAX_ZOOM = 2.5;
const FIT_MARGIN = 16;

export const clampZoom = (k: number): number => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, k));

/** `view` scaled by `factor` about the point (`px`, `py`) of the pane, which stays where it is under the pointer. */
export function zoomAt(view: Viewport, px: number, py: number, factor: number): Viewport {
    const k = clampZoom(view.k * factor);
    const f = k / view.k;
    return { k, x: px - (px - view.x) * f, y: py - (py - view.y) * f };
}

/**
 * `view` zoomed by `factor` with the whole map centred in the `w` by `h` pane at the new scale: what the zoom
 * buttons do, so the content is always in the middle of the pane after a click, never left off to one side.
 */
export function zoomCentered(
    view: Viewport,
    factor: number,
    width: number,
    height: number,
    w: number,
    h: number
): Viewport {
    const k = clampZoom(view.k * factor);
    return { k, x: (w - width * k) / 2, y: (h - height * k) / 2 };
}

/** Pixels a wheel "line" or "page" counts for, when a device reports its steps in them instead of pixels. */
const LINE_PIXELS = 16;
const PAGE_PIXELS = 400;

/**
 * The zoom factor of one wheel event. A mouse wheel steps in lines on some systems, which as raw numbers is a
 * tenth of a pixel step. (A wheel with Ctrl or Cmd is the pane zoom's, not the map's.)
 */
export function wheelFactor(e: { deltaY: number; deltaMode: number }): number {
    const pixels = e.deltaMode === 1 ? e.deltaY * LINE_PIXELS : e.deltaMode === 2 ? e.deltaY * PAGE_PIXELS : e.deltaY;
    return Math.exp(-pixels * 0.0015);
}

/**
 * The pointer's place in the map's own pixels. Inside a pane the person zoomed (the pane's own zoom is a CSS zoom)
 * the pointer's screen position and the map's pixels are not the same size, so the screen offset from the pane's
 * corner is divided by the ratio of the pane's drawn size to its own.
 */
export function localPoint(
    clientX: number,
    clientY: number,
    rect: { left: number; top: number; width: number; height: number },
    own: { w: number; h: number }
): { x: number; y: number } {
    const { sx, sy } = pointerScale(rect, own);
    return { x: (clientX - rect.left) / sx, y: (clientY - rect.top) / sy };
}

/**
 * How many screen pixels one of the map's own pixels is: 1 in a pane at its normal zoom, the pane zoom in one the
 * person zoomed. A pointer's travel on screen is divided by this to be the map's travel.
 */
export function pointerScale(
    rect: { width: number; height: number },
    own: { w: number; h: number }
): { sx: number; sy: number } {
    return {
        sx: own.w > 0 && rect.width > 0 ? rect.width / own.w : 1,
        sy: own.h > 0 && rect.height > 0 ? rect.height / own.h : 1,
    };
}

/**
 * The view that shows a `width` by `height` map whole in a `w` by `h` pane, no larger than life size, from the
 * top left: the map does not move about as agents come and go and it grows.
 */
export function fitView(
    width: number,
    height: number,
    w: number,
    h: number,
    limits: { min?: number; max?: number } = {}
): Viewport {
    if (width <= 0 || height <= 0 || w <= 0 || h <= 0) return { x: FIT_MARGIN, y: FIT_MARGIN, k: 1 };
    const fitted = Math.min((w - FIT_MARGIN * 2) / width, (h - FIT_MARGIN * 2) / height, limits.max ?? 1);
    // `min` is the readable floor: under it the map fills the pane the short way and is panned the other.
    const k = clampZoom(Math.max(fitted, limits.min ?? 0));
    return { k, x: FIT_MARGIN, y: FIT_MARGIN };
}
