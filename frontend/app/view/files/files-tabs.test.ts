// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Hangar on the app's pane tabs: a new tab opens beside this one in its pane,
 * closing a tab never closes the pane, and the pane's "+" starts a Hangar
 * tab in the folder the tab in front shows.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const lay = vi.hoisted(() => ({
    stacks: new Map<string, string[]>(),
    added: [] as unknown[],
    closed: [] as unknown[],
    meta: new Map<string, Record<string, unknown>>(),
}));

vi.mock("@/layout/lib/layoutModelHooks", () => ({
    getLayoutModelForStaticTab: () => ({
        getNodeByBlockId: (blockId: string) => {
            for (const [nodeId, stack] of lay.stacks) if (stack.includes(blockId)) return { id: nodeId, data: { blockStack: stack } };
            return undefined;
        },
    }),
}));
vi.mock("@/layout/lib/layoutStack", () => ({
    effectiveStack: (data: { blockStack: string[] }) => data.blockStack,
    addWidgetAsPaneTab: async (_m: unknown, nodeId: string, def: unknown) => {
        lay.added.push({ nodeId, def });
    },
    closeBlockInStack: async (_m: unknown, nodeId: string, blockId: string) => {
        lay.closed.push({ nodeId, blockId });
    },
}));
vi.mock("@/app/store/mos", async (orig) => ({
    ...(await orig<typeof import("@/app/store/mos")>()),
    getObjectValue: (oref: string | null) => (oref ? { meta: lay.meta.get(oref.replace(/^block:/, "")) } : undefined),
}));

import { closeOwnTab, openFolderInNewTab } from "./files-open";
import { filesPaneTab } from "./files";

beforeEach(() => {
    lay.stacks = new Map([["leaf-1", ["h1", "h2"]], ["leaf-2", ["solo"]]]);
    lay.added = [];
    lay.closed = [];
    lay.meta = new Map();
});

describe("Hangar pane tabs", () => {
    it("opens a folder in a new Hangar tab of the same pane", async () => {
        await openFolderInNewTab("h1", "C:\\work\\src");
        expect(lay.added).toEqual([{ nodeId: "leaf-1", def: { meta: { view: "files", "files:path": "C:\\work\\src" } } }]);
    });

    it("closes its own tab, but never the pane's last one", async () => {
        expect(await closeOwnTab("h2")).toBe(true);
        expect(lay.closed).toEqual([{ nodeId: "leaf-1", blockId: "h2" }]);
        expect(await closeOwnTab("solo")).toBe(false);
        expect(lay.closed).toHaveLength(1);
    });

    it("the pane's + starts a Hangar tab in the folder in front", () => {
        lay.meta.set("h2", { view: "files", "files:path": "C:\\work\\docs" });
        const chrome = filesPaneTab.chrome!("h1", { activeBlockId: () => "h2", blockId: "h1" } as never);
        expect(chrome.newTabMeta!("files")).toEqual({ "files:path": "C:\\work\\docs" });
        // Another kind of tab from the same "+" isn't given a folder.
        expect(chrome.newTabMeta!("term")).toBeUndefined();
    });
});
