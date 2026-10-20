// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A floater torn off with several tabs redocks as ONE pane
// (docs/analysis/RCA_WHOLE_PANE_TEAROFF_MOVES_ONLY_THE_VISIBLE_TAB_2026_10_19.md).

import { beforeEach, describe, expect, it, vi } from "vitest";

const { svc, state } = vi.hoisted(() => ({
    svc: { RedockFloatingPane: vi.fn() },
    state: { blockids: [] as string[], root: undefined as unknown },
}));

vi.mock("@/app/store/services", () => ({ WorkspaceService: svc }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (t: string, id: string) => `${t}:${id}`,
    getObjectValue: () => ({ blockids: state.blockids }),
}));
vi.mock("@/layout/lib/layoutModelHooks", () => ({
    getLayoutModelForTabById: () => ({ treeState: { rootNode: state.root } }),
}));

import { floaterPanes, redockFloaterPane } from "./floater-panes";

const pane = (data: Record<string, unknown>) => ({ id: "p", data });

beforeEach(() => {
    svc.RedockFloatingPane.mockReset().mockResolvedValue({ redocked: true });
    state.blockids = [];
    state.root = undefined;
});

describe("floaterPanes", () => {
    it("groups a torn-off stack into one pane, first tab first, with its visible tab", () => {
        state.blockids = ["b", "a", "c"]; // the visible tab is the tab's first block
        state.root = pane({ blockId: "b", blockStack: ["a", "b", "c"], activeBlockId: "b" });
        expect(floaterPanes("t")).toEqual([{ blockId: "a", members: ["a", "b", "c"], activeBlockId: "b" }]);
    });

    it("keeps single-tab panes separate and in tab order", () => {
        state.blockids = ["x", "y"];
        state.root = { id: "r", children: [pane({ blockId: "x" }), pane({ blockId: "y" })] };
        expect(floaterPanes("t")).toEqual([
            { blockId: "x", members: ["x"], activeBlockId: "x" },
            { blockId: "y", members: ["y"], activeBlockId: "y" },
        ]);
    });

    it("treats a block with no leaf yet as a one-tab pane", () => {
        state.blockids = ["x"];
        expect(floaterPanes("t")).toEqual([{ blockId: "x", members: ["x"], activeBlockId: "x" }]);
    });

    it("ignores stack members that are not in the floater's tab", () => {
        state.blockids = ["a", "b"];
        state.root = pane({ blockId: "a", blockStack: ["a", "b", "gone"], activeBlockId: "a" });
        expect(floaterPanes("t")).toEqual([{ blockId: "a", members: ["a", "b"], activeBlockId: "a" }]);
    });
});

describe("redockFloaterPane", () => {
    const src = { tabId: "ft", workspaceId: "fw" };
    const dst = { tabId: "tt", workspaceId: "tw" };

    it("makes the same call as before for a one-tab pane", async () => {
        await redockFloaterPane({ blockId: "a", members: ["a"], activeBlockId: "a" }, src, dst);
        expect(svc.RedockFloatingPane).toHaveBeenCalledWith("a", "ft", "fw", "tt", "tw");
        await redockFloaterPane({ blockId: "a", members: ["a"], activeBlockId: "a" }, src, dst, { blockId: "c", dir: 3 });
        expect(svc.RedockFloatingPane).toHaveBeenLastCalledWith("a", "ft", "fw", "tt", "tw", "c", 3);
    });

    it("sends every tab and the visible one for a stacked pane", async () => {
        await redockFloaterPane({ blockId: "a", members: ["a", "b"], activeBlockId: "b" }, src, dst, {
            blockId: "c",
            dir: 1,
        });
        expect(svc.RedockFloatingPane).toHaveBeenCalledWith("a", "ft", "fw", "tt", "tw", "c", 1, false, ["a", "b"], "b");
    });
});
