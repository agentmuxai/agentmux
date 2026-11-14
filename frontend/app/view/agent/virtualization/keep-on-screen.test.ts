// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LayoutView } from "@/app/store/agent-pane-layout-store";
import type { DocumentNode } from "../../../agent-model/types";
import { atBottomOf, keptScrollTop, onScreenRows } from "./keep-on-screen";

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

    it("knows a viewport at the bottom, within the follow controller's reattach distance", () => {
        const v = view(["a", "b", "c", "d"]); // 416 px of rows
        expect(atBottomOf(v, 116, 300)).toBe(true); // flush with the end
        expect(atBottomOf(v, 100, 300)).toBe(true); // 16 px short
        expect(atBottomOf(v, 50, 300)).toBe(false);
        expect(atBottomOf(null, 0, 300)).toBe(false);
    });
    describe("a change of view (SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.6)", () => {
        const run = (id: string, hiddenIds: string[]): DocumentNode =>
            ({ type: "hidden_run", id, counts: [], failed: 0, open: false, hiddenIds }) as DocumentNode;

        it("keeps the run that now holds the topmost row in place, not a row further down", () => {
            // a..f; the viewport shows c (offset 42) to f. The view hides a and b into one run, c and d into another.
            const rows = onScreenRows(view(["a", "b", "c", "d", "e", "f"]), 250, 300);
            const after = view(["hidden-run:a", "hidden-run:c", "e", "f"]);
            const nodes = [run("hidden-run:a", ["a", "b"]), run("hidden-run:c", ["c", "d"])];
            // The second run sits at 104 and keeps c's 42: the top of the pane does not move.
            expect(keptScrollTop(rows, after, 250, nodes)).toBe(146);
            // Without the runs in the nodes, the first row still there (e) is held instead.
            expect(keptScrollTop(rows, after, 250)).toBe(42);
        });

        it("keeps the first row of a run that opened out in place", () => {
            const rows = onScreenRows(view(["a", "hidden-run:b", "e", "f"]), 120, 300); // the run at offset 16
            const after = view(["x", "a", "b", "c", "d", "e", "f"]); // b starts at 208
            expect(keptScrollTop(rows, after, 120, [])).toBe(224);
        });

        it("keeps the offset within a run shorter than the row it replaced", () => {
            const rows = [{ nodeId: "c", offsetPx: 90 }];
            const after = { ...view(["a", "hidden-run:c"]) };
            after.rows[1] = { ...after.rows[1], height: 24, end: after.rows[1].start + 24 };
            expect(keptScrollTop(rows, after, 0, [run("hidden-run:c", ["c"])])).toBe(128);
        });
    });
});
