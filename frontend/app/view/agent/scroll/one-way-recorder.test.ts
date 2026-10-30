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

    it("rows moving down because the content below shrank are a tight shift, not a violation", () => {
        // A 120 px preview collapsed at the bottom: the browser clamped scrollTop, rows above slid down.
        const prev = frame(pinned, { scrollHeight: 1600, scrollTop: 1000 });
        const cur = frame([row("a", 120, 200), row("b", 320, 200), row("c", 520, 80)], { scrollHeight: 1480, scrollTop: 880 });
        const v = checkFrame(prev, cur).violation!;
        expect(v.kind).toBe("tight");
        expect(v.causes[0]).toBe("content-shrink:120");
    });

    it("rows moving down because the scroller got taller are a tight shift too", () => {
        const prev = frame(pinned);
        const cur = frame(pinned.map((r) => ({ ...r, top: r.top + 60, bottom: r.bottom + 60 })), { clientHeight: 660, scrollTop: 940 });
        const v = checkFrame(prev, cur).violation!;
        expect(v.kind).toBe("tight");
        expect(v.causes).toContain("viewport-grow:60");
    });

    it("a move down larger than the shrink that explains it is still a violation", () => {
        // A 1 px shrink cannot account for rows sliding 100 px.
        const down = pinned.map((r) => ({ ...r, top: r.top + 100, bottom: r.bottom + 100 }));
        const small = checkFrame(frame(pinned), frame(down, { scrollHeight: 1599, scrollTop: 900 })).violation!;
        expect(small.kind).toBe("down");
        // 120 px of shrink explains 120 px, not 200.
        const far = pinned.map((r) => ({ ...r, top: r.top + 200, bottom: r.bottom + 200 }));
        const big = checkFrame(frame(pinned), frame(far, { scrollHeight: 1480, scrollTop: 800 })).violation!;
        expect(big.kind).toBe("down");
        expect(big.causes[0]).toBe("content-shrink:120");
    });

    it("a shrink and a taller scroller together account for the sum of both", () => {
        const down = pinned.map((r) => ({ ...r, top: r.top + 150, bottom: r.bottom + 150 }));
        const v = checkFrame(frame(pinned), frame(down, { scrollHeight: 1540, clientHeight: 690, scrollTop: 850 })).violation!;
        expect(v.kind).toBe("tight");
    });

    it("growth above the visible rows does not hide a shrink below them", () => {
        // Eight 200 px rows, the last three visible. The first grows 50 px and the last shrinks 100 px:
        // the net change is -50, but the visible rows legitimately slide down by 100.
        const prev = frame([row("f", 0, 200), row("g", 200, 200), row("h", 400, 200)]);
        const cur = frame([row("f", 100, 200), row("g", 300, 200), row("h", 500, 100)], { scrollHeight: 1550, scrollTop: 950 });
        const v = checkFrame(prev, cur).violation!;
        expect(v.kind).toBe("tight");
        expect(v.dy).toBe(100);
    });

    it("a move down with no allowed cause is still a violation", () => {
        const down = pinned.map((r) => ({ ...r, top: r.top + 30, bottom: r.bottom + 30 }));
        expect(checkFrame(frame(pinned), frame(down, { scrollTop: 970 })).violation!.kind).toBe("down");
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

    it("names the row that grew", () => {
        const prev = [row("t", 0, 120, "tool"), row("b", 120, 200)];
        const cur = [row("t", 0, 150, "tool"), row("b", 150, 200)];
        expect(inferCauses(frame(prev), frame(cur, { scrollTop: 1010 }))).toContain("row-grow:tool:30");
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
