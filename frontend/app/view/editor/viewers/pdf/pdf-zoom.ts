// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The PDF viewer's zoom. The pane's zoom (`term:zoom`) is a multiple of the width that fits the pane,
 * so 100% is fit-to-width as the viewer opens, and pdf.js draws the pages again at the new scale
 * (vectors, not a stretched bitmap). What is under the pointer stays under it: the held point is a
 * page and a fraction of it, since pdf.js lays the pages out again at every scale. Rects are zoomed
 * px (chrome zoom) and `scrollTop` unzoomed. docs/specs/SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §10.
 */

import type { ZoomAnchorAdapter } from "@/app/store/zoom-anchor";
import { anchorPoint, TOLERANCE_PX } from "@/app/store/zoom-anchor-dom";

/** The most the PDF zooms in to, as a multiple of the width that fits (the shared top is 2). pdf.js stops at a scale of 25. */
export const PDF_ZOOM_MAX = 15;

/** One step of PDF zoom (a wheel notch, Ctrl +/-, a zoom button) multiplies the scale by this; tune it here. */
export const PDF_ZOOM_STEP_RATIO = 1.25;

/** A zoom step this small (as a ratio) is the scale already shown. */
const SAME_SCALE = 0.01;

/** pdf.js scale for the pane's `zoom`, a multiple of the scale that fits the width. */
export const scaleFor = (fitWidthScale: number, zoom: number): number => fitWidthScale * zoom;

/** The ratio to hand pdf.js's `updateScale`, or null when the pages already are at `target`. */
export function scaleFactorTo(current: number, target: number): number | null {
    if (!(current > 0) || !(target > 0)) return null;
    const ratio = target / current;
    return Math.abs(ratio - 1) < SAME_SCALE ? null : ratio;
}

/** The pane zoom at which pdf.js's `fitScale` shows (the "whole page" button), given the width fit. */
export const zoomForScale = (fitScale: number, fitWidthScale: number): number =>
    fitWidthScale > 0 ? fitScale / fitWidthScale : 1;

interface PdfAnchor {
    page: number;
    /** How far across and down the page the point was: 0 at the top left, 1 at the bottom right. */
    fx: number;
    fy: number;
    /** Where the point was in the scroller, zoomed px. */
    viewportX: number;
    viewportY: number;
}

export interface PdfZoomAnchorDeps {
    container: () => HTMLElement | undefined;
    /** The zoom the Editor's chrome is drawn at, which the pages are inside. */
    chromeZoom: () => number;
}

const pagesOf = (container: HTMLElement): HTMLElement[] => Array.from(container.querySelectorAll<HTMLElement>(".page"));

/** The page whose box holds `y`, else the nearest above it: pages stack top to bottom, so bisect. */
function pageAt(pages: HTMLElement[], y: number): HTMLElement | null {
    let lo = 0;
    let hi = pages.length - 1;
    let found: HTMLElement | null = pages[0] ?? null;
    while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        const rect = pages[mid].getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
        if (rect.top <= y) {
            found = pages[mid];
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    return found;
}

export function pdfZoomAnchor(deps: PdfZoomAnchorDeps): ZoomAnchorAdapter<PdfAnchor> {
    return {
        capture(point) {
            const container = deps.container();
            if (!container) return null;
            const box = container.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
            if (box.height <= 0) return null;
            const { x, y } = anchorPoint(box, point);
            const page = pageAt(pagesOf(container), y);
            if (!page) return null;
            const rect = page.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
            if (rect.width <= 0 || rect.height <= 0) return null;
            return {
                page: Number(page.dataset.pageNumber),
                fx: (x - rect.left) / rect.width,
                fy: (y - rect.top) / rect.height,
                viewportX: x - box.left,
                viewportY: y - box.top,
            };
        },

        restore(anchor) {
            const container = deps.container();
            const page = container?.querySelector<HTMLElement>(`.page[data-page-number="${anchor.page}"]`);
            if (!container || !page) return true;
            const rect = page.getBoundingClientRect(); // perf:allow-layout-read — settle passes, a few frames per zoom gesture
            const box = container.getBoundingClientRect(); // perf:allow-layout-read — settle passes, a few frames per zoom gesture
            const dx = rect.left + anchor.fx * rect.width - (box.left + anchor.viewportX);
            const dy = rect.top + anchor.fy * rect.height - (box.top + anchor.viewportY);
            if (Math.abs(dx) < TOLERANCE_PX && Math.abs(dy) < TOLERANCE_PX) return true;
            const zoom = deps.chromeZoom() || 1;
            container.scrollLeft += dx / zoom;
            container.scrollTop += dy / zoom;
            return false;
        },
    };
}
