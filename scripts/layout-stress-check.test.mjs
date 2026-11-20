// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { disagreements } from "./layout-stress-check.mjs";

const leaf = (id, extra = {}) => ({ id, data: { blockId: `b-${id}` }, ...extra });
const tile = (id, rect, extra = {}) => ({ id, minimized: false, hiddenByMaximize: false, blockId: `b-${id}`, rect, ...extra });
const area = [0, 0, 1000, 600];

describe("disagreements", () => {
    it("is empty when the window shows what is stored", () => {
        const dom = { tiles: [tile("a", [0, 0, 500, 600]), tile("b", [500, 0, 500, 600])], maximizedBlockId: null, area };
        const layout = { leaves: [leaf("a"), leaf("b")], magnifiednodeid: "", focusednodeid: "a" };
        expect(disagreements(dom, layout)).toEqual([]);
    });

    it("names a pane drawn but not stored, and one stored but not drawn", () => {
        const dom = { tiles: [tile("a", [0, 0, 500, 600]), tile("c", [500, 0, 500, 600])], maximizedBlockId: null, area };
        const layout = { leaves: [leaf("a"), leaf("b")], magnifiednodeid: "", focusednodeid: "" };
        const out = disagreements(dom, layout).join("\n");
        expect(out).toContain("c is drawn but not stored");
        expect(out).toContain("b is stored but not drawn");
    });

    it("names a minimized flag the window and the store disagree on", () => {
        const dom = { tiles: [tile("a", [0, 0, 1000, 600]), tile("b", [0, 0, 100, 30], { minimized: true })], maximizedBlockId: null, area };
        const layout = { leaves: [leaf("a"), leaf("b")], magnifiednodeid: "", focusednodeid: "" };
        expect(disagreements(dom, layout).join()).toContain("drawn minimized, stored expanded");
    });

    it("wants a maximized pane drawn exactly when one is stored, and the same one", () => {
        const layout = { leaves: [leaf("a"), leaf("b")], magnifiednodeid: "a", focusednodeid: "" };
        const none = { tiles: [tile("a", [0, 0, 500, 600]), tile("b", [500, 0, 500, 600])], maximizedBlockId: null, area };
        expect(disagreements(none, layout).join()).toContain("stored maximized and none is drawn");
        const other = { tiles: none.tiles, maximizedBlockId: "b-b", area };
        expect(disagreements(other, layout).join()).toContain("not the one stored");
        const ok = { tiles: none.tiles, maximizedBlockId: "b-a", area };
        expect(disagreements(ok, layout)).toEqual([]);
    });

    it("wants one pane expanded", () => {
        const dom = { tiles: [tile("a", [0, 0, 100, 30], { minimized: true })], maximizedBlockId: null, area };
        const layout = { leaves: [leaf("a", { minimized: true })], magnifiednodeid: "", focusednodeid: "" };
        expect(disagreements(dom, layout).join()).toContain("every stored pane is minimized");
    });

    it("finds expanded panes that overlap, leave the area or have no size", () => {
        const layout = { leaves: [leaf("a"), leaf("b"), leaf("c")], magnifiednodeid: "", focusednodeid: "" };
        const dom = { tiles: [tile("a", [0, 0, 600, 600]), tile("b", [500, 0, 500, 600]), tile("c", [990, 0, 15, 600])], maximizedBlockId: null, area };
        const out = disagreements(dom, layout).join("\n");
        expect(out).toContain("overlap by 100x600");
        expect(out).toContain("c is 15x600");
        expect(out).toContain("leaves the layout area");
    });

    it("does not judge geometry while a pane is maximized", () => {
        const layout = { leaves: [leaf("a"), leaf("b")], magnifiednodeid: "a", focusednodeid: "" };
        const dom = { tiles: [tile("a", [0, 0, 0, 0], { hiddenByMaximize: true }), tile("b", [0, 0, 0, 0], { hiddenByMaximize: true })], maximizedBlockId: "b-a", area };
        expect(disagreements(dom, layout)).toEqual([]);
    });
});
