// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The Editor's side of zoom that keeps the text under the pointer in place (`@/app/store/zoom-anchor`).
 * What is held is a document position, not a DOM node: CodeMirror redraws the lines it shows as it
 * re-measures, so a held node is often gone. Each pass asks the view (`coordsAtPos`) where the position
 * is now and scrolls by the difference; a position not drawn is placed by the height map, an estimate
 * the next pass corrects. Rects are zoomed px, line heights and `scrollTop` unzoomed. Spec §4.3, §9.
 */

import type { ZoomAnchorAdapter } from "@/app/store/zoom-anchor";
import { anchorPoint, TOLERANCE_PX } from "@/app/store/zoom-anchor-dom";

/** The parts of a CodeMirror `EditorView` the anchor uses. */
export interface ZoomEditorView {
    scrollDOM: HTMLElement;
    /** The document's top, in client px (zoomed). */
    documentTop: number;
    posAtCoords(coords: { x: number; y: number }, precise: false): number;
    coordsAtPos(pos: number): { top: number; bottom: number } | null;
    lineBlockAt(pos: number): { top: number; height: number };
}

interface EditorAnchor {
    pos: number;
    /** How far down its line the pointer was: 0 at the top, 1 at the bottom. */
    glyphY: number;
    /** How far below the scroller's top the pointer was, zoomed px. */
    viewportY: number;
}

export interface EditorZoomAnchorDeps {
    view: () => ZoomEditorView | null | undefined;
    /** The pane's zoom now: after the change when restoring. */
    zoom: () => number;
}

/** Where `pos` is on screen: its top and height in client px. */
function placeOf(view: ZoomEditorView, pos: number, zoom: number): { top: number; height: number } | null {
    try {
        const rect = view.coordsAtPos(pos);
        if (rect && rect.bottom > rect.top) return { top: rect.top, height: rect.bottom - rect.top };
        const block = view.lineBlockAt(pos);
        return { top: view.documentTop + block.top * zoom, height: block.height * zoom };
    } catch {
        return null; // the document got shorter than the position was
    }
}

export function editorZoomAnchor(deps: EditorZoomAnchorDeps): ZoomAnchorAdapter<EditorAnchor> {
    return {
        capture(point) {
            const view = deps.view();
            if (!view) return null;
            const box = view.scrollDOM.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
            if (box.height <= 0) return null;
            const { x, y } = anchorPoint(box, point);
            const pos = view.posAtCoords({ x, y }, false);
            const at = placeOf(view, pos, deps.zoom() || 1);
            if (!at) return null;
            return {
                pos,
                glyphY: at.height > 0 ? Math.min(1, Math.max(0, (y - at.top) / at.height)) : 0,
                viewportY: y - box.top,
            };
        },

        restore(anchor) {
            const view = deps.view();
            if (!view) return true;
            const at = placeOf(view, anchor.pos, deps.zoom() || 1);
            if (!at) return true;
            const box = view.scrollDOM.getBoundingClientRect(); // perf:allow-layout-read — settle passes, a few frames per zoom gesture
            const have = at.top + anchor.glyphY * at.height;
            const want = box.top + anchor.viewportY;
            if (Math.abs(have - want) < TOLERANCE_PX) return true;
            view.scrollDOM.scrollTop += (have - want) / (deps.zoom() || 1);
            return false;
        },
    };
}
