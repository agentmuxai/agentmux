// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The split shortcuts (Cmd+D, Shift+Cmd+D, Ctrl+Shift+S then an arrow) split
 * an agent pane into a fresh picker, like the pane menu does; any other pane
 * still splits into the default new block. Cmd+N is not a split.
 * SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const { g, blocks, focused } = vi.hoisted(() => ({
    g: { createBlock: vi.fn(), createBlockSplitHorizontally: vi.fn(), createBlockSplitVertically: vi.fn() },
    blocks: {} as Record<string, { meta: Record<string, unknown> }>,
    focused: { blockId: null as string | null },
}));

vi.mock("@/app/store/global", () => ({
    ...g,
    getSettingsKeyAtom: () => () => undefined,
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getMuxObjectAtom: (id: string) => () => blocks[id],
    },
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({
        focusedNode: () => (focused.blockId ? { data: { blockId: focused.blockId } } : null),
    }),
}));

import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
import { handleCmdN, handleSplitHorizontal, handleSplitVertical } from "./keymodel-blockcreate";

const PICKER = { meta: { view: "agentkb", controller: "cmd" } };
registerPaneTab(stubPaneTab("agentkb", { capabilities: { splitBlockDef: () => PICKER } }));
registerPaneTab(stubPaneTab("termkb", { capabilities: { sharesCwd: true } }));

beforeEach(() => {
    vi.clearAllMocks();
    blocks.agent = { meta: { view: "agentkb", agentId: "lark", "agent:runtime": { model: "opus" } } };
    blocks.term = { meta: { view: "termkb", "cmd:cwd": "C:/work" } };
});

describe("split shortcuts", () => {
    it("an agent pane splits into its declared picker, in every direction", async () => {
        focused.blockId = "agent";
        await handleSplitVertical("after");
        await handleSplitVertical("before");
        await handleSplitHorizontal("after");
        await handleSplitHorizontal("before");
        expect(g.createBlockSplitVertically.mock.calls).toEqual([
            [PICKER, "agent", "after"],
            [PICKER, "agent", "before"],
        ]);
        expect(g.createBlockSplitHorizontally.mock.calls).toEqual([
            [PICKER, "agent", "after"],
            [PICKER, "agent", "before"],
        ]);
    });

    it("any other pane still splits into the default new block (a terminal in its cwd)", async () => {
        focused.blockId = "term";
        await handleSplitVertical("after");
        expect(g.createBlockSplitVertically).toHaveBeenCalledWith(
            { meta: { view: "term", controller: "shell", "cmd:cwd": "C:/work" } },
            "term",
            "after"
        );
    });

    it("nothing focused: no split", async () => {
        focused.blockId = null;
        await handleSplitVertical("after");
        expect(g.createBlockSplitVertically).not.toHaveBeenCalled();
    });
});

describe("Cmd+N (a new block, not a split)", () => {
    it("still opens the default new block while an agent pane is focused", async () => {
        focused.blockId = "agent";
        await handleCmdN();
        expect(g.createBlock).toHaveBeenCalledWith({ meta: { view: "term", controller: "shell" } });
    });
});
