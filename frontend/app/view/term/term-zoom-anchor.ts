// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The terminal's side of zoom that keeps what is under the pointer in place (`@/app/store/zoom-anchor`).
 * A terminal zooms by its font size, and the refit that follows changes the rows and, through the
 * columns, rewraps the lines, so a buffer line number from before the zoom no longer names the same
 * text. What is held is an xterm marker, which follows its line through a rewrap and a trim; after the
 * zoom the viewport is scrolled so the marker is at the pixel height the pointer was at.
 * SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.3.
 */

import type { ZoomAnchorAdapter } from "@/app/store/zoom-anchor";
import { anchorPoint } from "@/app/store/zoom-anchor-dom";

/** The parts of an xterm `Terminal` the anchor uses. */
export interface ZoomTerminal {
    element?: HTMLElement;
    rows: number;
    buffer: { active: { viewportY: number; baseY: number; cursorY: number } };
    registerMarker(cursorYOffset: number): { line: number; isDisposed: boolean; dispose(): void } | undefined;
    scrollToLine(line: number): void;
}

interface TermAnchor {
    marker: NonNullable<ReturnType<ZoomTerminal["registerMarker"]>>;
    /** How far below the top of the screen the pointer was, px. */
    y: number;
}

export interface TermZoomAnchorDeps {
    terminal: () => ZoomTerminal | undefined;
    /** The viewport is on the last line: it stays there through a refit, and nothing is held. */
    following: () => boolean;
}

const screenOf = (t: ZoomTerminal): HTMLElement | null => t.element?.querySelector<HTMLElement>(".xterm-screen") ?? null;

export function termZoomAnchor(deps: TermZoomAnchorDeps): ZoomAnchorAdapter<TermAnchor> {
    let held: TermAnchor | undefined;
    return {
        capture(point) {
            held?.marker.dispose();
            held = undefined;
            const term = deps.terminal();
            const screen = term && screenOf(term);
            if (!term || !screen || deps.following()) return null;
            const box = screen.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
            if (box.height <= 0 || term.rows <= 0) return null;
            const { y } = anchorPoint(box, point);
            const row = Math.min(term.rows - 1, Math.max(0, Math.floor((y - box.top) / (box.height / term.rows))));
            const b = term.buffer.active;
            const marker = term.registerMarker(b.viewportY + row - (b.baseY + b.cursorY));
            if (!marker) return null;
            held = { marker, y: y - box.top };
            return held;
        },

        restore(anchor) {
            const term = deps.terminal();
            const screen = term && screenOf(term);
            if (!term || !screen || anchor.marker.isDisposed || anchor.marker.line < 0) return true;
            const box = screen.getBoundingClientRect(); // perf:allow-layout-read — settle passes, a few frames per zoom gesture
            if (box.height <= 0 || term.rows <= 0) return true;
            const b = term.buffer.active;
            const row = Math.min(term.rows - 1, Math.max(0, Math.floor(anchor.y / (box.height / term.rows))));
            const top = Math.min(b.baseY, Math.max(0, anchor.marker.line - row));
            if (top === b.viewportY) return true;
            term.scrollToLine(top);
            return false;
        },
    };
}
