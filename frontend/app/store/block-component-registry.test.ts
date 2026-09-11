// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for Codex P1/P2 on PR #3187: once a pane can keep
 * multiple stack members' `<Block>`s mounted simultaneously (terminal
 * keep-alive, `pane-leaf-chrome.tsx`), every registered blockId is no
 * longer automatically "the one currently-visible pane" — a dormant,
 * hidden stack member registers here too. `getAllBlockComponentModels`/
 * `getAllBlockComponentModelEntries` must still expose only the ACTIVE
 * member of each leaf, since every consumer (multi-input broadcast,
 * all-panes zoom, the multi-input-eligible terminal count) assumes that.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

const fakeNodesByBlockId = new Map<string, { data: { blockId: string } }>();

vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({
        getNodeByBlockId: (blockId: string) => fakeNodesByBlockId.get(blockId) ?? null,
    }),
}));

async function loadRegistry() {
    return import("./block-component-registry");
}

afterEach(() => {
    vi.resetModules();
    fakeNodesByBlockId.clear();
});

describe("getAllBlockComponentModels / getAllBlockComponentModelEntries", () => {
    it("excludes a dormant (non-active) stack member from the same leaf", async () => {
        const { registerBlockComponentModel, getAllBlockComponentModels, getAllBlockComponentModelEntries } =
            await loadRegistry();
        // Both "b1" and "b2" resolve to the SAME leaf (a stacked terminal
        // pane, keep-alive keeps both mounted+registered), but the leaf's
        // own `data.blockId` — reassigned to whichever member is active on
        // every switch (layoutStack.ts) — currently reads "b1".
        fakeNodesByBlockId.set("b1", { data: { blockId: "b1" } });
        fakeNodesByBlockId.set("b2", { data: { blockId: "b1" } });
        registerBlockComponentModel("b1", { viewModel: { viewType: "term" } } as any);
        registerBlockComponentModel("b2", { viewModel: { viewType: "term" } } as any);

        expect(getAllBlockComponentModels()).toHaveLength(1);
        expect(getAllBlockComponentModelEntries().map(([id]) => id)).toEqual(["b1"]);
    });

    it("includes a block once it becomes the active stack member", async () => {
        const { registerBlockComponentModel, getAllBlockComponentModelEntries } = await loadRegistry();
        fakeNodesByBlockId.set("b1", { data: { blockId: "b1" } });
        fakeNodesByBlockId.set("b2", { data: { blockId: "b1" } });
        registerBlockComponentModel("b1", { viewModel: { viewType: "term" } } as any);
        registerBlockComponentModel("b2", { viewModel: { viewType: "term" } } as any);

        // Switch: the leaf's own blockId now points at "b2".
        fakeNodesByBlockId.set("b1", { data: { blockId: "b2" } });
        fakeNodesByBlockId.set("b2", { data: { blockId: "b2" } });

        expect(getAllBlockComponentModelEntries().map(([id]) => id)).toEqual(["b2"]);
    });

    it("includes a registered block that isn't part of any stack at all (a plain, never-split pane)", async () => {
        const { registerBlockComponentModel, getAllBlockComponentModels } = await loadRegistry();
        fakeNodesByBlockId.set("b1", { data: { blockId: "b1" } });
        registerBlockComponentModel("b1", { viewModel: { viewType: "agent" } } as any);

        expect(getAllBlockComponentModels()).toHaveLength(1);
    });
});
