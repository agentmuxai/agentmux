// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The keys of a folder tree (tree-keys.ts): the table both the Editor's tree and the Files pane's tree view answer to. */

import { describe, expect, it } from "vitest";
import { treeMove, type KeyRow } from "./tree-keys";

// docs/ (open) > a.md, guide/ (closed); src/ (closed); readme
const rows: KeyRow[] = [
    { id: "docs", expandable: true, expanded: true, parent: null },
    { id: "docs/a.md", expandable: false, expanded: false, parent: "docs" },
    { id: "docs/guide", expandable: true, expanded: false, parent: "docs" },
    { id: "src", expandable: true, expanded: false, parent: null },
    { id: "readme", expandable: false, expanded: false, parent: null },
];

describe("treeMove", () => {
    it("ignores a key that is not a navigation key", () => {
        expect(treeMove(rows, "docs", "a", 5)).toBeNull();
        expect(treeMove(rows, "docs", "Enter", 5)).toBeNull();
    });

    it("moves over the rows, and stops at the ends", () => {
        expect(treeMove(rows, "docs", "ArrowDown", 5)).toEqual({ kind: "select", id: "docs/a.md" });
        expect(treeMove(rows, "src", "ArrowUp", 5)).toEqual({ kind: "select", id: "docs/guide" });
        expect(treeMove(rows, "docs", "ArrowUp", 5)).toEqual({ kind: "select", id: "docs" });
        expect(treeMove(rows, "readme", "ArrowDown", 5)).toEqual({ kind: "select", id: "readme" });
        expect(treeMove(rows, "src", "Home", 5)).toEqual({ kind: "select", id: "docs" });
        expect(treeMove(rows, "src", "End", 5)).toEqual({ kind: "select", id: "readme" });
        expect(treeMove(rows, "docs", "PageDown", 2)).toEqual({ kind: "select", id: "docs/guide" });
        expect(treeMove(rows, "readme", "PageUp", 2)).toEqual({ kind: "select", id: "docs/guide" });
    });

    it("starts at the first row when nothing is selected (End at the last)", () => {
        expect(treeMove(rows, null, "ArrowDown", 5)).toEqual({ kind: "select", id: "docs" });
        expect(treeMove(rows, null, "End", 5)).toEqual({ kind: "select", id: "readme" });
        expect(treeMove(rows, "gone", "ArrowDown", 5)).toEqual({ kind: "select", id: "docs" });
    });

    it("does nothing with no rows", () => {
        expect(treeMove([], null, "ArrowDown", 5)).toEqual({ kind: "none" });
    });

    it("Right opens a closed folder, steps into an open one, and does nothing on a file", () => {
        expect(treeMove(rows, "src", "ArrowRight", 5)).toEqual({ kind: "expand", id: "src" });
        expect(treeMove(rows, "docs", "ArrowRight", 5)).toEqual({ kind: "select", id: "docs/a.md" });
        expect(treeMove(rows, "readme", "ArrowRight", 5)).toEqual({ kind: "none" });
    });

    it("Right on an open folder whose rows haven't come yet does nothing", () => {
        const loading: KeyRow[] = [{ id: "docs", expandable: true, expanded: true, parent: null }, { id: "src", expandable: true, expanded: false, parent: null }];
        expect(treeMove(loading, "docs", "ArrowRight", 5)).toEqual({ kind: "none" });
    });

    it("Left closes an open folder, else steps out to the folder that holds the row, and does nothing at the top", () => {
        expect(treeMove(rows, "docs", "ArrowLeft", 5)).toEqual({ kind: "collapse", id: "docs" });
        expect(treeMove(rows, "docs/a.md", "ArrowLeft", 5)).toEqual({ kind: "select", id: "docs" });
        expect(treeMove(rows, "docs/guide", "ArrowLeft", 5)).toEqual({ kind: "select", id: "docs" });
        expect(treeMove(rows, "src", "ArrowLeft", 5)).toEqual({ kind: "none" });
    });
});
