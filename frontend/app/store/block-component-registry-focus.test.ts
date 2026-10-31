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
const revealBlock = vi.hoisted(() => vi.fn(async (_id: string, _opts?: unknown) => ({ ok: true }) as { ok: boolean; reason?: string }));

vi.mock("@/layout/index", () => ({ getLayoutModelForStaticTab: () => layout.model }));
vi.mock("./block-layout-actions", () => ({ createBlock }));
vi.mock("./shortcuts-recent", () => ({ recordBlockOpened: vi.fn() }));
vi.mock("./block-meta", () => ({ setBlockMeta: vi.fn(async () => {}) }));
vi.mock("./focusManager", () => ({ giveBlockFocus: (id: string) => void calls.push(`give:${id}`) }));
vi.mock("./block-atom-cache", () => ({ cleanupBlockAtomCache: vi.fn() }));
/** Panes open in the window but not mounted (an inactive tab of a pane's stack), by view. */
const unmounted = vi.hoisted(() => new Map<string, string>());
vi.mock("@/app/util/reveal-block", () => ({
    findOpenBlockByView: async (view: string) => unmounted.get(view) ?? null,
    revealBlock: async (id: string, opts?: unknown) => {
        calls.push(`reveal:${id}`);
        return revealBlock(id, opts);
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
        revealBlock.mockClear();
        revealBlock.mockResolvedValue({ ok: true });
        createBlock.mockClear();
    });
    afterEach(() => {
        layout.nodes.clear();
        unmounted.clear();
        for (const id of ["warden-1", "mem-1", "settings-1"]) unregisterBlockComponentModel(id);
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
        expect(revealBlock).not.toHaveBeenCalled();
        unregisterBlockComponentModel("warden-2");
    });

    it("opens a new pane when the pane in another tab cannot be shown, or there is none", async () => {
        register("warden-1", "warden");
        revealBlock.mockResolvedValue({ ok: false, reason: "not-found" });
        expect(await openOrFocusPaneIdByView("warden")).toBe("new-block");
        unregisterBlockComponentModel("warden-1");
        createBlock.mockClear();
        expect(await openOrFocusPaneIdByView("warden")).toBe("new-block");
        expect(createBlock).toHaveBeenCalledTimes(1);
        expect(showBlockInPane).not.toHaveBeenCalled();
    });

    it("keeps the pane in another tab when it was only slow or a newer click took over, instead of opening a copy", async () => {
        register("warden-1", "warden");
        for (const reason of ["superseded", "layout-timeout", "mismatch"]) {
            revealBlock.mockResolvedValue({ ok: false, reason });
            expect(await openOrFocusPaneIdByView("warden")).toBe("warden-1");
        }
        expect(createBlock).not.toHaveBeenCalled();
        revealBlock.mockResolvedValue({ ok: false, reason: "gone" });
        expect(await openOrFocusPaneIdByView("warden")).toBe("new-block");
    });

    it("brings an inactive tab of a pane's stack to the front instead of opening a copy of it", async () => {
        unmounted.set("memory", "mem-stacked"); // open, but unmounted: not in the component registry
        expect(await openOrFocusPaneIdByView("memory")).toBe("mem-stacked");
        expect(calls).toEqual(["reveal:mem-stacked"]);
        expect(revealBlock).toHaveBeenCalledWith("mem-stacked", { source: "widget", localOnly: true });
        expect(createBlock).not.toHaveBeenCalled();
    });

    it("opens a new pane only when the window has none, mounted or not", async () => {
        expect(await openOrFocusPaneIdByView("memory")).toBe("new-block");
        expect(createBlock).toHaveBeenCalledTimes(1);
    });
    it("opens one Settings pane in this window however many times it is asked, and shows it each time after", async () => {
        // The first ask creates it (the pane then mounts); the next two find it in this tab.
        createBlock.mockImplementationOnce(async () => {
            register("settings-1", "settings");
            layout.nodes.set("settings-1", { id: "node-s" });
            return "settings-1";
        });
        const def = { meta: { view: "settings", "settings:section": "browser" } };
        const focusMeta = { "settings:section": "browser" };
        for (let click = 0; click < 3; click++) expect(await openOrFocusPaneIdByView("settings", def as any, focusMeta as any)).toBe("settings-1");
        expect(createBlock).toHaveBeenCalledTimes(1);
        expect(calls.filter((c) => c.startsWith("focus:"))).toEqual(["focus:node-s", "focus:node-s"]);
    });
});
