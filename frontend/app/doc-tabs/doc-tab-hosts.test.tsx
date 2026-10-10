// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Moving a document tab between two panes of the same type: the registry,
 * the one move path (both sides asked before anything changes), the
 * controller host, and the pane-wide drop zone.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §3.3.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ dropTargets: [] as any[], cantMove: vi.fn() }));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    dropTargetForElements: (config: any) => {
        hub.dropTargets.push(config);
        return () => {};
    },
}));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantMove: hub.cantMove } }));

import { docTabItemType } from "@/app/drag/drag-types";
import { DocTabsController, type DocTabsSpec } from "./doc-tabs-controller";
import {
    ALREADY_OPEN_THERE,
    controllerHost,
    docTabHost,
    dropDocTab,
    moveDocTab,
    registerDocTabDropZone,
    registerDocTabHost,
    type DocTabHost,
} from "./doc-tab-hosts";

type P = { path: string };
const spec: DocTabsSpec<P> = {
    keyOf: (p) => p.path,
    titleOf: (p) => p.path.slice(1),
    serialize: (p) => p.path,
    deserialize: (st) => (typeof st === "string" ? { path: st } : null),
};
const host = () => ({ meta: () => ({}), setMeta: () => {} });
const keys = (ctl: DocTabsController<P>) => ctl.tabs().map((t) => t.key);

const unregister: (() => void)[] = [];
function pane(blockId: string, paths: string[], opts: Parameters<typeof controllerHost<P>>[2] = {}, docType = "media") {
    const ctl = new DocTabsController(spec, host(), paths.map((path) => ({ path })));
    unregister.push(registerDocTabHost(blockId, controllerHost(ctl, docType, opts)));
    return ctl;
}

beforeEach(() => {
    hub.dropTargets.length = 0;
    hub.cantMove.mockReset();
});
afterEach(() => {
    for (const u of unregister.splice(0)) u();
});

describe("moving a document tab between panes", () => {
    it("moves a tab, with its id, into the other pane after its active tab, and brings it to the front", () => {
        const a = pane("a", ["/1", "/2"]);
        const b = pane("b", ["/x", "/y"]);
        b.activate(b.tabs()[0].id);
        const moving = a.tabs()[1];
        expect(moveDocTab("a", moving.id, "b")).toEqual({ moved: true });
        expect(keys(a)).toEqual(["/1"]);
        expect(keys(b)).toEqual(["/x", "/2", "/y"]);
        expect(b.activeId()).toBe(moving.id);
        // Moved, not closed: the source can't reopen it.
        expect(a.reopen()).toBe(false);
    });

    it("lands beside the tab it was dropped on", () => {
        const a = pane("a", ["/1"]);
        const b = pane("b", ["/x", "/y"]);
        moveDocTab("a", a.tabs()[0].id, "b", { targetId: b.tabs()[0].id, position: "before" });
        expect(keys(b)).toEqual(["/1", "/x", "/y"]);
    });

    it("a document already open in the target stays where it is, and says why", () => {
        const a = pane("a", ["/1"]);
        const b = pane("b", ["/1", "/x"]);
        expect(moveDocTab("a", a.tabs()[0].id, "b")).toEqual({ moved: false, reason: ALREADY_OPEN_THERE });
        expect(keys(a)).toEqual(["/1"]);
        expect(keys(b)).toEqual(["/1", "/x"]);
    });

    it("nothing changes when either side refuses, and the reason comes back", () => {
        const a = pane("a", ["/1"], { refuseGive: () => "It has no file yet." });
        const b = pane("b", ["/x"]);
        expect(moveDocTab("a", a.tabs()[0].id, "b")).toEqual({ moved: false, reason: "It has no file yet." });
        expect(keys(a)).toEqual(["/1"]);
        expect(keys(b)).toEqual(["/x"]);

        const c = pane("c", ["/2"]);
        const refusing: DocTabHost = { ...docTabHost("b")!, refuseTake: () => "Not here." };
        unregister.push(registerDocTabHost("d", refusing));
        expect(moveDocTab("c", c.tabs()[0].id, "d")).toEqual({ moved: false, reason: "Not here." });
        expect(keys(c)).toEqual(["/2"]);
    });

    it("never between types, within one pane, from or to a pane that is gone, or for a tab that is gone", () => {
        const a = pane("a", ["/1"]);
        pane("e", ["/x"], {}, "editor");
        const id = a.tabs()[0].id;
        expect(moveDocTab("a", id, "e").moved).toBe(false);
        expect(moveDocTab("a", id, "a").moved).toBe(false);
        expect(moveDocTab("a", id, "nowhere").moved).toBe(false);
        expect(moveDocTab("nowhere", id, "a").moved).toBe(false);
        expect(moveDocTab("a", "no-such-tab", "e").moved).toBe(false);
        expect(keys(a)).toEqual(["/1"]);
    });

    it("a refused drop tells the user why; an allowed one says nothing", () => {
        const a = pane("a", ["/1", "/2"], { refuseGive: (t) => (t.key === "/1" ? "Stays." : null) });
        pane("b", ["/x"]);
        expect(dropDocTab("a", a.tabs()[0].id, "b")).toBe(false);
        expect(hub.cantMove).toHaveBeenCalledWith("1", "Stays.");
        hub.cantMove.mockReset();
        expect(dropDocTab("a", a.tabs()[1].id, "b")).toBe(true);
        expect(hub.cantMove).not.toHaveBeenCalled();
    });

    it("a pane's unregister removes only its own registration", () => {
        const first = controllerHost(new DocTabsController(spec, host(), []), "media");
        const second = controllerHost(new DocTabsController(spec, host(), []), "media");
        const off1 = registerDocTabHost("k", first);
        const off2 = registerDocTabHost("k", second);
        off1();
        expect(docTabHost("k")).toBe(second);
        off2();
        expect(docTabHost("k")).toBeUndefined();
    });
});

describe("the pane-wide drop zone", () => {
    const drag = (tabId: string, sourceBlockId: string, docType = "media") => ({
        source: { data: { type: docTabItemType, tabId, docType, sourceBlockId } },
    });

    it("takes a tab of its type from another pane, and declines its own, other types and other drags", () => {
        const el = document.createElement("div");
        registerDocTabDropZone(el, "media", "b");
        const zone = hub.dropTargets[0];
        expect(zone.element).toBe(el);
        expect(zone.canDrop(drag("t", "a"))).toBe(true);
        expect(zone.canDrop(drag("t", "b"))).toBe(false);
        expect(zone.canDrop(drag("t", "a", "editor"))).toBe(false);
        expect(zone.canDrop({ source: { data: { type: "PANE_TAB_ITEM", blockId: "x" } } })).toBe(false);
    });

    /** pragmatic-dnd's drop location: the accepting targets, innermost first. */
    const innermost = (...elements: HTMLElement[]) => ({ current: { dropTargets: elements.map((element) => ({ element })) } });

    it("a drop on one of its strip's tabs is that tab's alone", async () => {
        const a = pane("a", ["/1"]);
        const b = pane("b", ["/x"]);
        const el = document.createElement("div");
        const pill = document.createElement("div");
        el.appendChild(pill);
        registerDocTabDropZone(el, "media", "b");
        hub.dropTargets[0].onDrop({ ...drag(a.tabs()[0].id, "a"), location: innermost(pill, el) });
        await new Promise((r) => setTimeout(r, 0));
        expect(keys(a)).toEqual(["/1"]);
        expect(keys(b)).toEqual(["/x"]);
    });

    it("shows the drop look while one hovers, and moves the tab one task after the drop", async () => {
        const a = pane("a", ["/1"]);
        const b = pane("b", ["/x"]);
        const el = document.createElement("div");
        const cleanup = registerDocTabDropZone(el, "media", "b");
        const zone = hub.dropTargets[0];
        zone.onDragEnter();
        expect(el.classList.contains("doc-tab-host--drop-hover")).toBe(true);
        zone.onDrop({ ...drag(a.tabs()[0].id, "a"), location: innermost(el) });
        expect(el.classList.contains("doc-tab-host--drop-hover")).toBe(false);
        expect(keys(b)).toEqual(["/x"]);
        await new Promise((r) => setTimeout(r, 0));
        expect(keys(b)).toEqual(["/x", "/1"]);
        cleanup();
    });
});
