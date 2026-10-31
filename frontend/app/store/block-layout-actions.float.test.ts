// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const treeReducer = vi.fn();
const createBlockService = vi.fn((..._args: unknown[]) => Promise.resolve("docked-1"));
const openBlockFloating = vi.fn((..._args: unknown[]) => Promise.resolve<string | null>("float-1"));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ treeReducer, newEphemeralNode: vi.fn() }),
    LayoutTreeActionType: { InsertNode: "insert", SplitHorizontal: "sh", SplitVertical: "sv", ReplaceNode: "rn" },
    newLayoutNode: () => ({}),
}));
vi.mock("@/layout/lib/types", () => ({}));
vi.mock("./services", () => ({ ObjectService: { CreateBlock: (...a: unknown[]) => createBlockService(...a) } }));
vi.mock("./shortcuts-recent", () => ({ recordBlockOpened: vi.fn() }));
vi.mock("./open-floating", () => ({ openBlockFloating: (...a: unknown[]) => openBlockFloating(...a) }));
vi.mock("@/util/platformutil", () => ({ isModKey: (e: { ctrlKey: boolean }) => e.ctrlKey }));
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => true }));

import { createBlock } from "./block-layout-actions";
import { noteClick } from "./float-modifier";

const ctrl = { ctrlKey: true, metaKey: false, shiftKey: false, altKey: false };
const plain = { ctrlKey: false, metaKey: false, shiftKey: false, altKey: false };
const def = { meta: { view: "term" } } as BlockDef;

beforeEach(() => {
    vi.clearAllMocks();
    openBlockFloating.mockResolvedValue("float-1");
    noteClick(plain);
});

describe("createBlock after a Ctrl+click", () => {
    it("opens a floating pane and no docked one", async () => {
        noteClick(ctrl);
        expect(await createBlock(def)).toBe("float-1");
        expect(openBlockFloating).toHaveBeenCalledWith(def);
        expect(createBlockService).not.toHaveBeenCalled();
        expect(treeReducer).not.toHaveBeenCalled();
    });

    it("docks the next open, which the click did not cause", async () => {
        noteClick(ctrl);
        await createBlock(def);
        expect(await createBlock(def)).toBe("docked-1");
        expect(treeReducer).toHaveBeenCalledTimes(1);
    });

    it("docks as usual after a plain click", async () => {
        noteClick(plain);
        expect(await createBlock(def)).toBe("docked-1");
        expect(openBlockFloating).not.toHaveBeenCalled();
    });

    it("docks when the block can't be opened floating", async () => {
        noteClick(ctrl);
        openBlockFloating.mockResolvedValue(null);
        expect(await createBlock(def)).toBe("docked-1");
    });
});
