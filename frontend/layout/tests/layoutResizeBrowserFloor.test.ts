// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A browser pane can't be dragged narrower than MinBrowserPaneWidthPx
// (SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md §5.5).

import { assert, test } from "vitest";
import {
    computeGroupResizeSizes,
    DefaultGapSizePx,
    MinBrowserPaneWidthPx,
    MinNodeSizePx,
    minExtentPx,
} from "../lib/layoutResize";
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
    const stacked = leaf("a", "term", { data: { blockId: "term", blockStack: ["term", "web"], activeBlockId: "term" } });
    assert.equal(minExtentPx(stacked, FlexDirection.Row, isBrowser), MinBrowserPaneWidthPx);
});

test("a group's floor adds up along its own direction and takes the widest across it", () => {
    const row: LayoutNode = {
        id: "r",
        flexDirection: FlexDirection.Row,
        size: 1,
        children: [leaf("a", "web"), leaf("b", "term")],
    };
    assert.equal(minExtentPx(row, FlexDirection.Row, isBrowser), MinBrowserPaneWidthPx + MinNodeSizePx + DefaultGapSizePx);
    const column: LayoutNode = { ...row, id: "c", flexDirection: FlexDirection.Column };
    assert.equal(minExtentPx(column, FlexDirection.Row, isBrowser), MinBrowserPaneWidthPx);
    assert.equal(minExtentPx(column, FlexDirection.Column, isBrowser), 2 * MinNodeSizePx + DefaultGapSizePx);
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
