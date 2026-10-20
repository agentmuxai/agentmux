// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A pane dragged out by its header tears off with ALL of its tabs
// (docs/analysis/RCA_WHOLE_PANE_TEAROFF_MOVES_ONLY_THE_VISIBLE_TAB_2026_10_19.md).

import { beforeEach, describe, expect, it, vi } from "vitest";

const { removeMovedBlock } = vi.hoisted(() => ({ removeMovedBlock: vi.fn() }));
vi.mock("@/layout/lib/layoutMagnify", () => ({ removeMovedBlock: (...a: unknown[]) => removeMovedBlock(...a) }));

import { paneTearOffOf, removeTornOffPane } from "./pane-tearoff";

const leaf = (data: Record<string, unknown>) => ({ id: "n1", data }) as never;

describe("paneTearOffOf", () => {
    it("carries every tab of a stacked pane in order, with the visible one as blockId", () => {
        const t = paneTearOffOf(leaf({ blockId: "b", blockStack: ["a", "b", "c"], activeBlockId: "b" }));
        expect(t).toEqual({ blockId: "b", members: ["a", "b", "c"] });
    });

    it("is just the block for a pane with one tab", () => {
        expect(paneTearOffOf(leaf({ blockId: "a" }))).toEqual({ blockId: "a", members: ["a"] });
        expect(paneTearOffOf(leaf({ blockId: "a", blockStack: ["a"], activeBlockId: "a" }))).toEqual({
            blockId: "a",
            members: ["a"],
        });
    });

    it("falls back to the visible block when a stale stack does not list it", () => {
        expect(paneTearOffOf(leaf({ blockId: "z", blockStack: ["a", "b"] }))).toEqual({ blockId: "z", members: ["z"] });
    });

    it("is null for a node with no block", () => {
        expect(paneTearOffOf(undefined)).toBeNull();
        expect(paneTearOffOf(leaf({ blockId: "" }))).toBeNull();
    });

    it("does not alias the node's own stack", () => {
        const data = { blockId: "a", blockStack: ["a", "b"] };
        const t = paneTearOffOf(leaf(data))!;
        t.members.push("x");
        expect(data.blockStack).toEqual(["a", "b"]);
    });
});

describe("removeTornOffPane", () => {
    beforeEach(() => removeMovedBlock.mockReset());

    it("removes each tab from the source layout, in order", () => {
        const model = {} as never;
        removeTornOffPane(model, ["a", "b", "c"]);
        expect(removeMovedBlock.mock.calls).toEqual([
            [model, "a"],
            [model, "b"],
            [model, "c"],
        ]);
    });
});
