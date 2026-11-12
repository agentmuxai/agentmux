// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the zoom amount is drawn: the centre of the pane being zoomed, held so the whole amount stays inside the
 * window. "All panes" and the chrome zoom name no pane and keep the window's centre (the stylesheet's 50% / 50%).
 */

export interface Box {
    x: number;
    y: number;
    width: number;
    height: number;
}

export interface Size {
    width: number;
    height: number;
}

export interface Point {
    x: number;
    y: number;
}

function clamp(value: number, low: number, high: number): number {
    return Math.min(Math.max(value, low), high);
}

/** One axis: the centre of `[start, start + length]` cut to the viewport, then moved so `size` fits in `[0, view]`. */
function centreOn(start: number, length: number, size: number, view: number): number | null {
    const lo = Math.max(start, 0);
    const hi = Math.min(start + length, view);
    if (hi <= lo) return null;
    // Bigger than the window: nothing fits, so the window's own centre.
    if (size >= view) return view / 2;
    return clamp((lo + hi) / 2, size / 2, view - size / 2);
}

/**
 * The point (viewport pixels) the indicator is centred on, or null for the window's centre: no pane, or a pane
 * that is not on screen. A pane partly outside the window counts by its visible part, a pane smaller than the
 * indicator by its centre moved the least that fits it inside the window.
 */
export function zoomIndicatorCenter(pane: Box | null, size: Size, viewport: Size): Point | null {
    if (pane == null) return null;
    const x = centreOn(pane.x, pane.width, size.width, viewport.width);
    const y = centreOn(pane.y, pane.height, size.height, viewport.height);
    return x == null || y == null ? null : { x, y };
}

/**
 * The body of a pane on screen, by its block id: the biggest element carrying the id (a pane's frame; the id
 * also sits on small parts and on copies in tabs that are not shown, which have no box), then its content area
 * so the header is not counted. Null when the pane has no box in this window.
 */
export function paneBodyBox(blockId: string, root: ParentNode = document): Box | null {
    let frame: Element | null = null;
    let frameArea = 0;
    for (const el of Array.from(root.querySelectorAll(`[data-blockid="${CSS.escape(blockId)}"]`))) {
        const r = el.getBoundingClientRect();
        if (r.width * r.height > frameArea) {
            frame = el;
            frameArea = r.width * r.height;
        }
    }
    if (frame == null) return null;
    const content = frame.querySelector(".block-content");
    const r = (content ?? frame).getBoundingClientRect();
    return r.width > 0 && r.height > 0 ? { x: r.x, y: r.y, width: r.width, height: r.height } : null;
}
