// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    moves: [] as any[],
    inLayout: new Set<string>(),
    tabOf: {} as Record<string, string>,
}));

vi.mock("@/layout/index", () => ({
    moveBlocksToTab: (move: any) => hub.moves.push(move),
    tabOfBlock: (blockId: string) => hub.tabOf[blockId],
}));
vi.mock("@/layout/lib/layoutNode", () => ({
    findNodeByBlockId: (_root: unknown, blockId: string) => (hub.inLayout.has(blockId) ? { id: "n" } : undefined),
}));

import { foreignTabMoves } from "./pane-foreign-tab";

const model = (rootNode: object | null = {}) =>
    ({ treeState: { rootNode }, tabAtom: () => ({ oid: "tab-here" }) }) as any;

beforeEach(() => {
    hub.moves = [];
    hub.inLayout = new Set(["mine"]);
    hub.tabOf = { theirs: "tab-away", other: "tab-away" };
});

describe("a pill from another window tab", () => {
    it("is foreign when no pane of this layout holds it, and not when one does", () => {
        const foreign = foreignTabMoves(model(), () => "pane-block");
        expect(foreign.isForeign("theirs")).toBe(true);
        expect(foreign.isForeign("mine")).toBe(false);
    });

    it("is never foreign before the layout has a tree to look in", () => {
        expect(foreignTabMoves(model(null), () => "pane-block").isForeign("theirs")).toBe(false);
    });

    it("joins the pane it was dropped on as a tab, through the one cross-tab move", () => {
        const foreign = foreignTabMoves(model(), () => "pane-block");
        expect(foreign.move(["theirs"], "theirs", { asTab: true })).toBe(false);
        expect(hub.moves).toEqual([
            { blockIds: ["theirs"], activeBlockId: "theirs", sourceTabId: "tab-away", targetTabId: "tab-here", targetBlockId: "pane-block", asTab: true },
        ]);
    });

    it("lands beside the pane when dropped on its body, several picked tabs together", () => {
        const foreign = foreignTabMoves(model(), () => "pane-block");
        foreign.move(["theirs", "other"], "other", { direction: 3 });
        expect(hub.moves).toEqual([
            { blockIds: ["theirs", "other"], activeBlockId: "other", sourceTabId: "tab-away", targetTabId: "tab-here", targetBlockId: "pane-block", direction: 3 },
        ]);
    });

    it("moves nothing when its tab or the pane it was dropped on is unknown", () => {
        foreignTabMoves(model(), () => "pane-block").move(["nowhere"], "nowhere", { asTab: true });
        foreignTabMoves(model(), () => undefined).move(["theirs"], "theirs", { asTab: true });
        expect(hub.moves).toEqual([]);
    });
});
