// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A browser pane can't be dragged narrower than MinBrowserPaneWidthPx
// (SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md §5.5).

import { assert, test } from "vitest";
import { computeGroupResizeSizes, MinBrowserPaneWidthPx, minExtentPx, MinNodeSizePx } from "../lib/layoutResize";
import { FlexDirection, type LayoutNode } from "../lib/types";

const browsers = new Set(["web", "web2"]);
const isBrowser = (id: string) => browsers.has(id);

function leaf(id: string, blockId: string, extra: Partial<LayoutNode> = {}): LayoutNode {
    return { id, data: { blockId }, flexDirection: FlexDirection.Row, size: 1, ...extra };
}

test("a browser pane's floor is 250px across and the usual floor down", () => {
    assert.equal(minExtentPx(leaf("a", "web"), FlexDirection.Row, isBrowser), MinBrowserPaneWidthPx);
    assert.equal(minExtentPx(leaf("a", "web"), FlexDirection.Column, isBrowser), MinNodeSizePx);
    assert.equal(minExtentPx(leaf("a", "term"), FlexDirection.Row, isBrowser), MinNodeSizePx);
});

test("a pane counts as a browser when any of its in-pane tabs is one", () => {
    const stacked = leaf("a", "term", {
        data: { blockId: "term", blockStack: ["term", "web"], activeBlockId: "term" },
    });
    assert.equal(minExtentPx(stacked, FlexDirection.Row, isBrowser), MinBrowserPaneWidthPx);
});

test("a group's floor follows its members' weights along its own direction and takes the widest across it", () => {
    const row: LayoutNode = {
        id: "r",
        flexDirection: FlexDirection.Row,
        size: 1,
        children: [leaf("a", "web"), leaf("b", "term")],
    };
    // Equal weights: the browser gets half the group, so the group needs twice its floor
    // (the terminal's 2 x 128 is less). Adding the floors up (381) left it at about 190.
    assert.equal(minExtentPx(row, FlexDirection.Row, isBrowser), 2 * MinBrowserPaneWidthPx);
    // A 3:1 row: the browser holds 3/4 of the group, the terminal 1/4 (needs 4 x 128).
    const skewed: LayoutNode = { ...row, children: [leaf("a", "web", { size: 3 }), leaf("b", "term", { size: 1 })] };
    // The browser needs 250 x 4/3 = 333; the terminal needs 4 x 128 = 512: the terminal decides.
    assert.equal(minExtentPx(skewed, FlexDirection.Row, isBrowser), 4 * MinNodeSizePx);
    const column: LayoutNode = { ...row, id: "c", flexDirection: FlexDirection.Column };
    assert.equal(minExtentPx(column, FlexDirection.Row, isBrowser), MinBrowserPaneWidthPx);
    assert.equal(minExtentPx(column, FlexDirection.Column, isBrowser), 2 * MinNodeSizePx);
});

test("a nested row inside a column keeps its browser at 250 when the column is at its floor", () => {
    const inner: LayoutNode = {
        id: "inner",
        flexDirection: FlexDirection.Row,
        size: 1,
        children: [leaf("a", "web"), leaf("b", "term")],
    };
    const outer: LayoutNode = {
        id: "outer",
        flexDirection: FlexDirection.Column,
        size: 1,
        children: [inner, leaf("c", "term")],
    };
    const floor = minExtentPx(outer, FlexDirection.Row, isBrowser);
    // At that width the browser is half the inner row: exactly its floor.
    assert.isAtLeast(floor / 2, MinBrowserPaneWidthPx);
});

test("a minimized member takes no room in its group's floor", () => {
    const row: LayoutNode = {
        id: "r",
        flexDirection: FlexDirection.Row,
        size: 1,
        children: [leaf("a", "web", { minimized: true }), leaf("b", "term")],
    };
    assert.equal(minExtentPx(row, FlexDirection.Row, isBrowser), MinNodeSizePx);
});

test("a drag stops a browser pane at its own floor while a terminal beside it goes to the usual one", () => {
    // 1px per unit: the browser (before the handle) and a terminal (after it).
    const floors = new Map([
        ["browser", MinBrowserPaneWidthPx],
        ["term", MinNodeSizePx],
    ]);
    const siblings = [
        { nodeId: "browser", size: 400 },
        { nodeId: "term", size: 400 },
    ];
    // Drag the handle far left: the terminal wants 700, the browser would be 100.
    const left = computeGroupResizeSizes(siblings, "term", 700, floors);
    assert.equal(left.get("browser"), MinBrowserPaneWidthPx);
    assert.equal(left.get("term"), 800 - MinBrowserPaneWidthPx);
    // Far right: the terminal stops at its own, smaller floor.
    const right = computeGroupResizeSizes(siblings, "term", 50, floors);
    assert.equal(right.get("term"), MinNodeSizePx);
    assert.equal(right.get("browser"), 800 - MinNodeSizePx);
});
