// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Zoom and pan for an image in the Editor's image viewer: the wheel zooms about the
// cursor, a click zooms a step in or out, a drag pans. Plain maths on the
// image's fitted box; image-viewer.tsx applies the result as a CSS transform
// (translate, then scale about the box's centre).

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

/** How many steps a click takes from fit to the deepest zoom. */
const CLICK_STEPS = 5;

/** The scales a click zooms through: fit, then five more, evenly spaced on a
 *  log scale up to the maximum, so each click looks like the same amount of
 *  zoom. docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §7. */
export function clickSteps(box: ZoomBox): number[] {
    const max = maxScale(box);
    const steps = [1];
    for (let k = 1; k < CLICK_STEPS; k++) steps.push(max ** (k / CLICK_STEPS));
    steps.push(max);
    return steps;
}

/** The next step strictly above (`direction` 1) or below (-1) `scale`, if
 *  any. A scale a rounding error off a step counts as on it. */
function nextStep(scale: number, direction: 1 | -1, box: ZoomBox): number | undefined {
    const steps = clickSteps(box);
    const beyond = (s: number) => (s - scale) * direction > scale * 1e-6;
    return direction > 0 ? steps.find(beyond) : steps.reverse().find(beyond);
}

/** One click: to the next step above or below the current scale, about the
 *  cursor. After a wheel zoom that is the nearest step that way, not a fixed
 *  jump; nothing past the deepest step or fit. */
export function zoomStep(
    state: ZoomState,
    direction: 1 | -1,
    cursor: { x: number; y: number },
    box: ZoomBox
): ZoomState {
    const target = nextStep(state.scale, direction, box);
    return target === undefined ? state : zoomAt(state, target / state.scale, cursor, box);
}

/** At the deepest step, where a click can't zoom in any further. */
export function atMaxZoom(state: ZoomState, box: ZoomBox): boolean {
    return nextStep(state.scale, 1, box) === undefined;
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
