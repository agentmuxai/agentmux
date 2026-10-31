// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The terminal's zoom anchor: a marker holds the line under the pointer through the refit, and the
 * viewport is scrolled so it is at the pointer's height again. A fake terminal stands in for xterm.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { termZoomAnchor, type ZoomTerminal } from "./term-zoom-anchor";

const rect = (top: number, height: number): DOMRect =>
    ({ top, bottom: top + height, left: 0, right: 800, width: 800, height, x: 0, y: top }) as DOMRect;

interface FakeMarker {
    line: number;
    isDisposed: boolean;
    dispose: () => void;
}

let screen: HTMLElement;
let term: ZoomTerminal & { buffer: { active: { viewportY: number; baseY: number; cursorY: number } }; markers: FakeMarker[] };
let following: boolean;

const adapter = () => termZoomAnchor({ terminal: () => term, following: () => following });

beforeEach(() => {
    document.body.innerHTML = `<div id="t"><div class="xterm-screen"></div></div>`;
    screen = document.querySelector(".xterm-screen") as HTMLElement;
    screen.getBoundingClientRect = () => rect(100, 400); // perf:allow-layout-read — a stub that stands in for the screen box
    following = false;
    const markers: FakeMarker[] = [];
    term = {
        element: document.getElementById("t")!,
        rows: 20,
        // 1000 lines of scrollback above a 20 row screen, the viewport at line 300.
        buffer: { active: { viewportY: 300, baseY: 1000, cursorY: 5 } },
        markers,
        registerMarker: (offset: number) => {
            const m: FakeMarker = {
                line: 1000 + 5 + offset,
                isDisposed: false,
                dispose: () => {
                    m.isDisposed = true;
                },
            };
            markers.push(m);
            return m;
        },
        scrollToLine: vi.fn((line: number) => {
            term.buffer.active.viewportY = line;
        }),
    };
});

describe("termZoomAnchor", () => {
    it("marks the line under the pointer and, after a refit, scrolls it back to the pointer's height", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 250 })!; // 150 px down: row 7 of 20 px rows
        expect(token.marker.line).toBe(307); // viewport line 300 + row 7

        // The font grew: 10 rows of 40 px, and the rewrap moved the marked line to 640.
        term.rows = 10;
        screen.getBoundingClientRect = () => rect(100, 400); // perf:allow-layout-read — a stub that stands in for the screen box
        token.marker.line = 640;
        expect(a.restore(token)).toBe(false);
        expect(term.scrollToLine).toHaveBeenCalledWith(637); // 150 px / 40 px = row 3
        expect(a.restore(token)).toBe(true); // in place now
    });

    it("clamps to the buffer: not above its first line, not past the last viewport", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 450 })!;
        token.marker.line = 2;
        a.restore(token);
        expect(term.scrollToLine).toHaveBeenLastCalledWith(0);
        token.marker.line = 5000;
        term.buffer.active.viewportY = 0;
        a.restore(token);
        expect(term.scrollToLine).toHaveBeenLastCalledWith(1000);
    });

    it("uses the screen's centre when the pointer is not over it", () => {
        const token = adapter().capture(null)!;
        expect(token.marker.line).toBe(310); // row 10
    });

    it("holds nothing at the bottom of the buffer (a refit keeps it there), or without a terminal or a layout", () => {
        following = true;
        expect(adapter().capture({ clientX: 50, clientY: 250 })).toBeNull();
        following = false;
        expect(termZoomAnchor({ terminal: () => undefined, following: () => false }).capture(null)).toBeNull();
        screen.getBoundingClientRect = () => rect(0, 0); // perf:allow-layout-read — a stub that stands in for the screen box
        expect(adapter().capture(null)).toBeNull();
    });

    it("lets go of the last marker at the next capture, and has nothing to do for a marker that is gone", () => {
        const a = adapter();
        const first = a.capture({ clientX: 50, clientY: 250 })!;
        a.capture({ clientX: 50, clientY: 250 });
        expect(first.marker.isDisposed).toBe(true);
        expect(a.restore(first)).toBe(true);
        expect(term.scrollToLine).not.toHaveBeenCalled();
    });
});
