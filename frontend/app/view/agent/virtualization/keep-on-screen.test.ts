// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LayoutView } from "@/app/store/agent-pane-layout-store";
import { keptScrollTop, onScreenRows } from "./keep-on-screen";

/** A view of rows `ids`, each `px` tall with a 4 px gap, windowed to all rows. */
function view(ids: string[], px = 100, margin = 0): LayoutView {
    const rows = ids.map((nodeId, index) => {
        const start = margin + index * (px + 4);
        return { nodeId, index, start, height: px, end: start + px };
    });
    return { rows, totalSize: rows.length * (px + 4), window: { startIndex: 0, endIndex: rows.length - 1 }, scrollMarginPx: margin };
}

describe("keep-on-screen (SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §4.1)", () => {
    it("lists the rows on screen, top first, with the viewport's offset into each", () => {
        // 10 rows of 104 px; the viewport shows 250..550: rows 2 (208..308) to 5 (520..620).
        const rows = onScreenRows(view(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]), 250, 300);
        expect(rows.map((r) => r.nodeId)).toEqual(["c", "d", "e", "f"]);
        expect(rows[0].offsetPx).toBe(42);
    });

    it("keeps the top row in place when rows above it are removed (a roll-off)", () => {
        const before = view(["a", "b", "c", "d", "e", "f"]);
        const rows = onScreenRows(before, 250, 300);
        const after = view(["c", "d", "e", "f"]); // a and b rolled off: c moves up 208 px
        expect(keptScrollTop(rows, after, 250)).toBe(42);
    });

    it("keeps it in place when a row is inserted above (the history link)", () => {
        const rows = onScreenRows(view(["a", "b", "c"]), 120, 200);
        const after = view(["link", "a", "b", "c"]);
        expect(keptScrollTop(rows, after, 120)).toBe(224);
    });

    it("uses the first row still there when the top one was removed", () => {
        const rows = onScreenRows(view(["a", "b", "c", "d"]), 50, 300); // a (offset 50), b, c
        const after = view(["c", "d"]); // a and b gone: c keeps its offset (50 - 208 = -158 → c starts at 0)
        expect(keptScrollTop(rows, after, 50)).toBe(0);
    });

    it("changes nothing for an append, or when nothing on screen survives", () => {
        const rows = onScreenRows(view(["a", "b", "c"]), 120, 200);
        expect(keptScrollTop(rows, view(["a", "b", "c", "d"]), 120)).toBeNull();
        expect(keptScrollTop(rows, view(["x", "y"]), 120)).toBeNull();
        expect(keptScrollTop([], view(["a"]), 0)).toBeNull();
        expect(onScreenRows(view(["a"]), 0, 0)).toEqual([]); // not laid out yet
    });
});
