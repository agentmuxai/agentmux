// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { paintRows, resizeAndPaint } from "./term-paint";

/** A terminal shaped like xterm's, recording what happened to it in order. */
function fakeTerminal(cols: number, rows: number, internals = true) {
    const calls: string[] = [];
    const t = {
        cols,
        rows,
        resize(c: number, r: number) {
            calls.push(`resize ${c}x${r}`);
            t.cols = c;
            t.rows = r;
        },
        _core: internals
            ? {
                  _renderService: {
                      clear: () => calls.push("clear"),
                      _renderRows: (start: number, end: number) => calls.push(`render ${start}-${end}`),
                  },
              }
            : undefined,
    };
    return { t, calls };
}

describe("resizeAndPaint", () => {
    it("paints every row of the new grid right after resizing, in the same call", () => {
        const { t, calls } = fakeTerminal(80, 24);
        expect(resizeAndPaint(t, { cols: 100, rows: 30 })).toBe(true);
        expect(calls).toEqual(["clear", "resize 100x30", "render 0-29"]);
    });

    it("does nothing when the grid is already that size", () => {
        const { t, calls } = fakeTerminal(80, 24);
        expect(resizeAndPaint(t, { cols: 80, rows: 24 })).toBe(false);
        expect(calls).toEqual([]);
    });

    it("still resizes when xterm's internals aren't there", () => {
        const { t, calls } = fakeTerminal(80, 24, false);
        expect(resizeAndPaint(t, { cols: 81, rows: 24 })).toBe(true);
        expect(calls).toEqual(["resize 81x24"]);
    });
});

describe("paintRows", () => {
    it("renders rows 0 to the last", () => {
        const { t, calls } = fakeTerminal(80, 24);
        paintRows(t);
        expect(calls).toEqual(["render 0-23"]);
    });
});
