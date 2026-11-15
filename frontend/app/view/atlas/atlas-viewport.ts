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
 * The view that shows a `width` by `height` map whole in a `w` by `h` pane, no larger than life size, from the
 * top left: the map does not move about as agents come and go and it grows.
 */
export function fitView(width: number, height: number, w: number, h: number): Viewport {
    if (width <= 0 || height <= 0 || w <= 0 || h <= 0) return { x: FIT_MARGIN, y: FIT_MARGIN, k: 1 };
    const k = clampZoom(Math.min((w - FIT_MARGIN * 2) / width, (h - FIT_MARGIN * 2) / height, 1));
    return { k, x: FIT_MARGIN, y: FIT_MARGIN };
}
