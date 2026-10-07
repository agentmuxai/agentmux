// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { blankGap, checkFrame, inferCauses, type FrameSample, type RowBox } from "./one-way-recorder";

const row = (id: string, top: number, height: number, type = "markdown"): RowBox => ({
    id,
    type,
    top,
    bottom: top + height,
});

const frame = (rows: RowBox[], over: Partial<FrameSample> = {}): FrameSample => ({
    t: 0,
    following: true,
    userInput: false,
    scrollTop: 1000,
    scrollHeight: 1600,
    clientHeight: 600,
    clientWidth: 800,
    zoom: 1,
    bottomPad: 0,
    rows,
    ...over,
});

// A pinned pane: three rows filling the 600 px viewport exactly.
const pinned = [row("a", 0, 200), row("b", 200, 200), row("c", 400, 200)];

describe("checkFrame — V1 (one direction)", () => {
    it("content moving up is fine", () => {
        const up = pinned.map((r) => ({ ...r, top: r.top - 40, bottom: r.bottom - 40 }));
        expect(checkFrame(frame(pinned), frame([...up, row("d", 560, 40)], { scrollTop: 1040 })).violation).toBeNull();
    });

    it("a visible row moving down is a violation, worst row reported", () => {
        const down = [row("a", 10, 200), row("b", 230, 200), row("c", 430, 200)];
        const v = checkFrame(frame(pinned), frame(down, { scrollTop: 970 })).violation!;
        expect(v.kind).toBe("down");
        expect(v.rowId).toBe("b");
        expect(v.dy).toBe(30);
        expect(v.rows).toBe(3);
        expect(v.causes).toContain("scroll-up:30");
    });

    it("sub-pixel settling is not a violation", () => {
        const nudge = pinned.map((r) => ({ ...r, top: r.top + 0.4, bottom: r.bottom + 0.4 }));
        expect(checkFrame(frame(pinned), frame(nudge)).violation).toBeNull();
    });

    it("rows off-screen in either frame are ignored", () => {
        const prev = [row("x", -300, 200), ...pinned];
        const cur = [row("x", -250, 200), ...pinned];
        expect(checkFrame(frame(prev), frame(cur)).violation).toBeNull();
    });

    it("is exempt while detached, under user input, or on a width or zoom change", () => {
        const down = pinned.map((r) => ({ ...r, top: r.top + 50, bottom: r.bottom + 50 }));
        expect(checkFrame(frame(pinned, { following: false }), frame(down)).violation).toBeNull();
        expect(checkFrame(frame(pinned), frame(down, { following: false })).violation).toBeNull();
        expect(checkFrame(frame(pinned), frame(down, { userInput: true })).violation).toBeNull();
        expect(checkFrame(frame(pinned, { userInput: true }), frame(down)).violation).toBeNull();
        expect(checkFrame(frame(pinned), frame(down, { clientWidth: 700 })).violation).toBeNull();
        expect(checkFrame(frame(pinned), frame(down, { zoom: 0.9 })).violation).toBeNull();
    });
});

describe("checkFrame — V2 (no overshoot)", () => {
    it("blank space below the last row is measured", () => {
        // The last row ends 40 px above the viewport bottom: a gap.
        const gapped = [row("a", 0, 200), row("b", 200, 200), row("c", 400, 160)];
        expect(blankGap(frame(gapped))).toBe(40);
        expect(blankGap(frame(pinned))).toBe(0);
    });

    it("padding-bottom is not a gap", () => {
        const padded = [row("a", 0, 200), row("b", 200, 200), row("c", 400, 196)];
        expect(blankGap(frame(padded, { bottomPad: 4 }))).toBe(0);
    });

    it("a non-overflowing pane has no gap", () => {
        expect(blankGap(frame([row("a", 0, 100)], { scrollHeight: 600 }))).toBe(0);
    });

    it("a gap then a downward move is an overshoot", () => {
        const gapped = [row("a", 0, 200), row("b", 200, 200), row("c", 400, 160)];
        const settled = gapped.map((r) => ({ ...r, top: r.top + 40, bottom: r.bottom + 40 }));
        const v = checkFrame(frame(gapped), frame(settled, { scrollTop: 960 })).violation!;
        expect(v.kind).toBe("overshoot");
        expect(v.gapBefore).toBe(40);
    });
});

describe("inferCauses", () => {
    it("names a viewport that grew", () => {
        expect(inferCauses(frame(pinned), frame(pinned, { clientHeight: 640 }))).toContain("viewport-grow:40");
    });

    it("names the row that shrank and the content shrink", () => {
        const prev = [row("a", 0, 200), row("t", 200, 300, "tool")];
        const cur = [row("a", 0, 200), row("t", 200, 120, "tool")];
        const causes = inferCauses(frame(prev), frame(cur, { scrollHeight: 1420 }));
        expect(causes).toContain("row-shrink:tool:180");
        expect(causes).toContain("content-shrink:180");
    });

    it("names a visible row that was removed", () => {
        expect(inferCauses(frame(pinned), frame(pinned.slice(0, 2)))).toContain("row-removed:1");
    });

    it("falls back to transform when only positions changed", () => {
        const moved = pinned.map((r) => ({ ...r, top: r.top + 6, bottom: r.bottom + 6 }));
        expect(inferCauses(frame(pinned), frame(moved))).toEqual(["transform"]);
    });

    it("appends the code's own notes to the inferred causes", () => {
        const down = pinned.map((r) => ({ ...r, top: r.top + 20, bottom: r.bottom + 20 }));
        const v = checkFrame(frame(pinned), frame(down), ["hold:release-ease"]).violation!;
        expect(v.causes).toEqual(["transform", "hold:release-ease"]);
    });
});
