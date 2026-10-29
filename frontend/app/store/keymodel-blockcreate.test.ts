// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The split shortcuts (Cmd+D, Shift+Cmd+D, Ctrl+Shift+S then an arrow) open a Shortcuts pane
 * next to the focused pane, like the pane menu does. Cmd+N is not a split.
 * SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const { g, blocks, focused, settingValue } = vi.hoisted(() => ({
    g: { createBlock: vi.fn(), createBlockSplitHorizontally: vi.fn(), createBlockSplitVertically: vi.fn() },
    blocks: {} as Record<string, { oid: string; meta: Record<string, unknown> }>,
    focused: { blockId: null as string | null },
    settingValue: { current: undefined as string | undefined },
}));

vi.mock("@/app/store/global", () => ({
    ...g,
    getSettingsKeyAtom: () => () => settingValue.current,
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
    blocks.agent = { oid: "agent", meta: { view: "agentkb", agentId: "lark", "agent:runtime": { model: "opus" } } };
    blocks.term = { oid: "term", meta: { view: "termkb", "cmd:cwd": "C:/work" } };
});

describe("split shortcuts", () => {
    const shortcuts = (from: string) => ({ meta: { view: "shortcuts", "shortcuts:from": from } });

    it("every pane splits into a Shortcuts pane naming it, in every direction (an agent pane too)", async () => {
        for (const id of ["agent", "term"]) {
            vi.clearAllMocks();
            focused.blockId = id;
            await handleSplitVertical("after");
            await handleSplitVertical("before");
            await handleSplitHorizontal("after");
            await handleSplitHorizontal("before");
            expect(g.createBlockSplitVertically.mock.calls).toEqual([
                [shortcuts(id), id, "after"],
                [shortcuts(id), id, "before"],
            ]);
            expect(g.createBlockSplitHorizontally.mock.calls).toEqual([
                [shortcuts(id), id, "after"],
                [shortcuts(id), id, "before"],
            ]);
        }
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

describe("app:defaultnewblock", () => {
    it("is not read by a split, and `launcher` and `shortcuts` both open the Shortcuts pane on Cmd+N", async () => {
        for (const value of ["launcher", "shortcuts"]) {
            vi.clearAllMocks();
            settingValue.current = value;
            focused.blockId = "agent";
            await handleCmdN();
            expect(g.createBlock).toHaveBeenCalledWith({ meta: { view: "shortcuts" } });
        }
        settingValue.current = undefined;
    });
});
