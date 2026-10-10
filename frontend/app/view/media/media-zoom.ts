// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Zoom and pan for an image in the Media view: the wheel zooms about the
// cursor, a drag pans, double-click toggles fit and actual size. Plain maths
// on the image's fitted box; media-view.tsx applies the result as a CSS
// transform (translate, then scale about the box's centre).

/** How the image is shown: `scale` times its fitted size, moved by `x`/`y`
 *  CSS pixels from the centre. `scale` 1 is "fit" (the default). */
export interface ZoomState {
    scale: number;
    x: number;
    y: number;
}

/** The sizes zoom works with, in the view's own CSS pixels. */
export interface ZoomBox {
    /** The image's fitted size (its box at scale 1). */
    fitWidth: number;
    fitHeight: number;
    /** The view it sits in. */
    viewWidth: number;
    viewHeight: number;
    /** The image's own pixels. */
    naturalWidth: number;
    naturalHeight: number;
}

export const FIT: ZoomState = { scale: 1, x: 0, y: 0 };

/** The most the image can be enlarged: 32 of its own pixels per CSS pixel,
 *  and never less than 4 times its fitted size. */
export function maxScale(box: ZoomBox): number {
    if (box.fitWidth <= 0) return 4;
    return Math.max(4, (32 * box.naturalWidth) / box.fitWidth);
}

/** The scale at which one image pixel is one CSS pixel. */
export function actualSizeScale(box: ZoomBox): number {
    return box.fitWidth > 0 ? box.naturalWidth / box.fitWidth : 1;
}

/** The zoom as a percentage of the image's own size, as viewers show it. */
export function zoomPercent(state: ZoomState, box: ZoomBox): number {
    return box.naturalWidth > 0 ? Math.round((100 * state.scale * box.fitWidth) / box.naturalWidth) : 100;
}

/** The scale factor for one wheel event: a notch (deltaY ±100 in pixels,
 *  or ±1 line / page) zooms by about 20%; a trackpad's small deltas zoom
 *  smoothly by as much as they scroll. */
export function wheelFactor(deltaY: number, deltaMode: number): number {
    const pixels = deltaMode === 1 ? deltaY * 40 : deltaMode === 2 ? deltaY * 400 : deltaY;
    // Capped so one fast flick can't jump from fit to the maximum.
    const step = Math.max(-300, Math.min(300, pixels));
    return Math.exp(-step * 0.0018);
}

/** Keeps the image in reach: at or below fit it is centred; enlarged, it can
 *  move until its edge reaches the view's edge, no further. */
export function clampPan(state: ZoomState, box: ZoomBox): ZoomState {
    const limitX = Math.max(0, (box.fitWidth * state.scale - box.viewWidth) / 2);
    const limitY = Math.max(0, (box.fitHeight * state.scale - box.viewHeight) / 2);
    return {
        scale: state.scale,
        x: Math.max(-limitX, Math.min(limitX, state.x)),
        y: Math.max(-limitY, Math.min(limitY, state.y)),
    };
}

/** `state` scaled by `factor` (clamped between fit and the maximum), with the
 *  image point under `cursor` staying under it. `cursor` is relative to the
 *  view's centre. */
export function zoomAt(state: ZoomState, factor: number, cursor: { x: number; y: number }, box: ZoomBox): ZoomState {
    const scale = Math.max(1, Math.min(maxScale(box), state.scale * factor));
    if (scale === state.scale) return state;
    if (scale === 1) return FIT;
    const ratio = scale / state.scale;
    return clampPan(
        {
            scale,
            x: cursor.x - ratio * (cursor.x - state.x),
            y: cursor.y - ratio * (cursor.y - state.y),
        },
        box
    );
}

/** Double-click: from fit, to actual size (or twice the fit, for an image no
 *  bigger than the view) at the cursor; from any zoom, back to fit. */
export function toggleZoom(state: ZoomState, cursor: { x: number; y: number }, box: ZoomBox): ZoomState {
    if (state.scale > 1) return FIT;
    const actual = actualSizeScale(box);
    const target = actual > 1.05 ? actual : 2;
    return zoomAt(state, target / state.scale, cursor, box);
}

/** Moved by a drag of `dx`/`dy` CSS pixels. */
export function panBy(state: ZoomState, dx: number, dy: number, box: ZoomBox): ZoomState {
    if (state.scale <= 1) return state;
    return clampPan({ scale: state.scale, x: state.x + dx, y: state.y + dy }, box);
}

/** Show the image's own pixels as squares, rather than blurred, once each
 *  is drawn larger than 2 CSS pixels: what you want to inspect a render. */
export function showsPixels(state: ZoomState, box: ZoomBox): boolean {
    return box.fitWidth > 0 && (state.scale * box.fitWidth) / box.naturalWidth > 2;
}
