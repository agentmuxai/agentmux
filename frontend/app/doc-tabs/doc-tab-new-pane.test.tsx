// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A document tab moved into a new pane beside another (doc-tab-new-pane.ts):
 * dropped near a pane's edge, or "Move to New Pane".
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §4 (Phase 4).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    splits: [] as { dir: "h" | "v"; def: unknown; beside: string; position: string }[],
    closed: [] as string[],
    cantMove: vi.fn(),
    dropTargets: [] as any[],
    nextId: 0,
}));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    dropTargetForElements: (config: any) => {
        h.dropTargets.push(config);
        return () => {};
    },
}));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantMove: h.cantMove } }));
vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}`, getObjectValue: () => ({ oid: "src", meta: { view: "media" } }) },
    createBlockSplitHorizontally: async (def: unknown, beside: string, position: string) => {
        h.splits.push({ dir: "h", def, beside, position });
        return `new${++h.nextId}`;
    },
    createBlockSplitVertically: async (def: unknown, beside: string, position: string) => {
        h.splits.push({ dir: "v", def, beside, position });
        return `new${++h.nextId}`;
    },
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({
        getNodeByBlockId: (id: string) => ({ id: `node-${id}` }),
        closeNode: async (nodeId: string) => void h.closed.push(nodeId),
    }),
}));

import { docTabItemType } from "@/app/drag/drag-types";
import { registerDocTabHost, type DocTabHost } from "./doc-tab-hosts";
import { moveDocTabToNewPane, registerDocTabSplitTarget } from "./doc-tab-new-pane";

const unregister: (() => void)[] = [];
/** A host holding tabs by id, that gives and takes them. */
function host(blockId: string, ids: string[], opts: Partial<DocTabHost> = {}) {
    const tabs = ids.map((id) => ({ id, key: id, title: id, preview: false, pinned: false, payload: {} }));
    const hst: DocTabHost = {
        docType: "media",
        peek: (id) => tabs.find((t) => t.id === id) ?? null,
        refuseGive: () => null,
        refuseTake: () => null,
        give: (id) => {
            const i = tabs.findIndex((t) => t.id === id);
            return i < 0 ? null : { docType: "media", tab: tabs.splice(i, 1)[0] };
        },
        take: (transfer) => void tabs.push(transfer.tab as (typeof tabs)[number]),
        ...opts,
    };
    unregister.push(registerDocTabHost(blockId, hst));
    return tabs;
}

beforeEach(() => {
    h.splits.length = 0;
    h.closed.length = 0;
    h.dropTargets.length = 0;
    h.cantMove.mockReset();
});
afterEach(() => {
    for (const u of unregister.splice(0)) u();
});

describe("moving a document tab into a new pane", () => {
    it("splits a pane of the tab's type off on that side, waits for it, and moves the tab in", async () => {
        const src = host("src", ["t1", "t2"]);
        const moved = moveDocTabToNewPane("src", "t2", "beside", "left");
        await Promise.resolve();
        await Promise.resolve();
        expect(h.splits).toEqual([{ dir: "h", def: { meta: { view: "media" } }, beside: "beside", position: "before" }]);
        // The new pane mounts a moment later.
        const target = host(`new${h.nextId}`, []);
        expect(await moved).toBe(true);
        expect(src.map((t) => t.id)).toEqual(["t1"]);
        expect(target.map((t) => t.id)).toEqual(["t2"]);
    });

    it("up and down split the other way, after for down", async () => {
        host("src", ["t1"]);
        const p = moveDocTabToNewPane("src", "t1", "beside", "down");
        await Promise.resolve();
        await Promise.resolve();
        host(`new${h.nextId}`, []);
        await p;
        expect(h.splits[0]).toMatchObject({ dir: "v", position: "after" });
    });

    it("a tab its pane won't give up makes no pane, and says why", async () => {
        host("src", ["t1"], { refuseGive: () => "It is still being saved." });
        expect(await moveDocTabToNewPane("src", "t1", "src", "right")).toBe(false);
        expect(h.splits).toEqual([]);
        expect(h.cantMove).toHaveBeenCalledWith("t1", "It is still being saved.");
    });

    it("a move refused once the pane is made removes that pane again", async () => {
        host("src", ["t1"]);
        const p = moveDocTabToNewPane("src", "t1", "src", "right");
        await Promise.resolve();
        await Promise.resolve();
        host(`new${h.nextId}`, [], { refuseTake: () => "No." });
        expect(await p).toBe(false);
        expect(h.closed).toEqual([`node-new${h.nextId}`]);
    });
});

describe("a pane's tile as a drop target for a document tab", () => {
    const tile = () => {
        const el = document.createElement("div");
        el.className = "tile-node";
        el.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) });
        return el;
    };
    const drag = (clientX: number, clientY: number) => ({
        source: { data: { type: docTabItemType, tabId: "t1", docType: "media", sourceBlockId: "src" } },
        input: { clientX, clientY },
    });

    it("takes a document tab only near its edge, and shows that side while it is the innermost target", () => {
        const el = tile();
        registerDocTabSplitTarget(el, () => "beside");
        const target = h.dropTargets[0];
        expect(target.canDrop(drag(5, 50))).toBe(true);
        expect(target.canDrop(drag(50, 50))).toBe(false);
        expect(target.canDrop({ source: { data: { type: "PANE_TAB_ITEM" } }, input: { clientX: 5, clientY: 50 } })).toBe(false);
        const at = (x: number, y: number, inner: Element) => ({ location: { current: { dropTargets: [{ element: inner }], input: { clientX: x, clientY: y } } } });
        target.onDrag(at(95, 50, el));
        expect(el.classList.contains("doc-tab-split--right")).toBe(true);
        target.onDrag(at(95, 50, document.createElement("div")));
        expect(el.classList.contains("doc-tab-split--right")).toBe(false);
        target.onDragLeave();
        expect(el.className).toBe("tile-node");
    });

    it("a drop near the edge moves the tab into a new pane on that side, on the next task", async () => {
        host("src", ["t1"]);
        const el = tile();
        registerDocTabSplitTarget(el, () => "beside");
        const target = h.dropTargets[0];
        target.onDrop({ ...drag(50, 3), location: { current: { dropTargets: [{ element: el }], input: { clientX: 50, clientY: 3 } } } });
        expect(h.splits).toEqual([]);
        await new Promise((r) => setTimeout(r, 0));
        await Promise.resolve();
        expect(h.splits[0]).toMatchObject({ dir: "v", beside: "beside", position: "before" });
    });
});
