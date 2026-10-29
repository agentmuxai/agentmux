// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The widget bar's Warden / Memory / Connectors entries focus the pane already open instead of
 * opening another. A pane the person minimized, or one that is a background tab of a pane's stack,
 * has to be shown first: `focusNode` alone leaves it collapsed or hidden.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
const layout = vi.hoisted(() => ({
    nodes: new Map<string, { id: string; minimized?: boolean }>(),
    model: {
        getNodeByBlockId: (blockId: string) => layout.nodes.get(blockId),
        focusNode: (id: string) => void calls.push(`focus:${id}`),
    },
}));
const createBlock = vi.hoisted(() => vi.fn(async () => "new-block"));
const showBlockInPane = vi.hoisted(() => vi.fn());
const revealBlockLocally = vi.hoisted(() => vi.fn(async (_id: string, _opts?: unknown) => true));

vi.mock("@/layout/index", () => ({ getLayoutModelForStaticTab: () => layout.model }));
vi.mock("./block-layout-actions", () => ({ createBlock }));
vi.mock("./block-meta", () => ({ setBlockMeta: vi.fn(async () => {}) }));
vi.mock("./focusManager", () => ({ giveBlockFocus: (id: string) => void calls.push(`give:${id}`) }));
vi.mock("./block-atom-cache", () => ({ cleanupBlockAtomCache: vi.fn() }));
vi.mock("@/app/util/reveal-block", () => ({
    revealBlockLocally: async (id: string, opts?: unknown) => {
        calls.push(`reveal:${id}`);
        return revealBlockLocally(id, opts);
    },
    showBlockInPane: (...args: unknown[]) => {
        calls.push(`show:${(args[1] as { id: string }).id}:${args[2]}`);
        showBlockInPane(...args);
    },
}));

import { openOrFocusPaneIdByView, registerBlockComponentModel, unregisterBlockComponentModel } from "./block-component-registry";

const register = (blockId: string, viewType: string) =>
    registerBlockComponentModel(blockId, { viewModel: { viewType, blockId } } as any);

describe("openOrFocusPaneIdByView", () => {
    beforeEach(() => {
        calls.length = 0;
        showBlockInPane.mockClear();
        revealBlockLocally.mockClear();
        revealBlockLocally.mockResolvedValue(true);
        createBlock.mockClear();
    });
    afterEach(() => {
        layout.nodes.clear();
        for (const id of ["warden-1", "mem-1"]) unregisterBlockComponentModel(id);
    });

    it("shows a minimized pane before it focuses it, instead of opening another", async () => {
        register("warden-1", "warden");
        layout.nodes.set("warden-1", { id: "node-w", minimized: true });
        expect(await openOrFocusPaneIdByView("warden")).toBe("warden-1");
        expect(calls).toEqual(["show:node-w:warden-1", "focus:node-w", "give:warden-1"]);
        expect(createBlock).not.toHaveBeenCalled();
    });

    it("shows a background member of a stack through the pane's node and its own block id", async () => {
        register("mem-1", "memory");
        layout.nodes.set("mem-1", { id: "stack-node" });
        expect(await openOrFocusPaneIdByView("memory")).toBe("mem-1");
        expect(calls[0]).toBe("show:stack-node:mem-1");
    });

    it("goes to a pane in another tab of this window instead of opening one here", async () => {
        register("warden-1", "warden");
        expect(await openOrFocusPaneIdByView("warden")).toBe("warden-1");
        expect(calls).toEqual(["reveal:warden-1"]);
        expect(createBlock).not.toHaveBeenCalled();
    });

    it("prefers the pane in this tab over one in another tab", async () => {
        register("warden-1", "warden");
        register("warden-2", "warden");
        layout.nodes.set("warden-2", { id: "node-2" });
        expect(await openOrFocusPaneIdByView("warden")).toBe("warden-2");
        expect(revealBlockLocally).not.toHaveBeenCalled();
        unregisterBlockComponentModel("warden-2");
    });

    it("opens a new pane when the pane in another tab cannot be shown, or there is none", async () => {
        register("warden-1", "warden");
        revealBlockLocally.mockResolvedValue(false);
        expect(await openOrFocusPaneIdByView("warden")).toBe("new-block");
        unregisterBlockComponentModel("warden-1");
        createBlock.mockClear();
        expect(await openOrFocusPaneIdByView("warden")).toBe("new-block");
        expect(createBlock).toHaveBeenCalledTimes(1);
        expect(showBlockInPane).not.toHaveBeenCalled();
    });
});
