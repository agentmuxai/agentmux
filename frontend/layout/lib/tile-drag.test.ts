// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A whole-pane (tile) drag's shared state, from start to release to end.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1, §5.8.
 */

import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    payloads: [] as unknown[],
    crossTabCleared: 0,
    dragsEnded: [] as unknown[],
    window: [] as string[],
    escaped: false,
}));
vi.mock("@/app/drag/native-cancel", () => ({ endedByEscape: async () => hub.escaped }));
vi.mock("./tearOffPreview", () => ({
    leaveWindow: (_m: unknown, id: string) => hub.window.push(`out:${id}`),
    returnToWindow: () => hub.window.push("in"),
}));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: (p: unknown) => hub.payloads.push(p),
}));
vi.mock("./crossTabDrag", () => ({ clearCrossTabDrop: () => hub.crossTabCleared++ }));
vi.mock("./layoutIntents", () => ({ layoutDragEnded: (m: unknown) => hub.dragsEnded.push(m) }));

import { beginDrag, endDrag, isUnderway, session } from "@/app/drag/drag-session";
import { endTileDrag, releaseTileDrag, startTileDrag } from "./tile-drag";
import { dragState } from "./tilelayout-drag-state";

function fakeModel(tabId = "tab-1") {
    const [get, set] = createSignal(false);
    return { activeDrag: Object.assign(get, { _set: set }), tabAtom: () => ({ oid: tabId }) } as any;
}
const node = { id: "node-1", data: { blockId: "b1" } } as any;

afterEach(() => {
    endDrag("cancel");
    hub.window = [];
    hub.escaped = false;
    hub.payloads = [];
    hub.crossTabCleared = 0;
});

describe("tile drag", () => {
    it("start records the drag everywhere it is read today, and begins a tile session", () => {
        const model = fakeModel();
        startTileDrag(node, model);
        expect(dragState).toMatchObject({ nodeId: "node-1", layoutModel: model, node });
        expect(model.activeDrag()).toBe(true);
        expect(hub.crossTabCleared).toBe(1);
        expect(hub.payloads).toEqual([{ kind: "tile", node, sourceTabId: "tab-1" }]);
        expect(session()).toMatchObject({ kind: "tile", source: { nodeId: "node-1", tabId: "tab-1" }, released: false });
        expect(isUnderway("tile")).toBe(true);
    });

    it("the source's release resets the local state but keeps the payload for the cross-window monitor", () => {
        const model = fakeModel();
        startTileDrag(node, model);
        releaseTileDrag(model);
        expect(dragState).toMatchObject({ nodeId: null, layoutModel: null, node: null });
        expect(model.activeDrag()).toBe(false);
        expect(hub.dragsEnded).toContain(model); // the comparison skipped mid-drag runs now
        expect(hub.payloads).not.toContain(null);
        expect(session()?.released).toBe(true);
        expect(isUnderway("tile")).toBe(false);
    });

    it("end-of-drag cleanup ends the tile session", () => {
        startTileDrag(node, fakeModel());
        endTileDrag("drop");
        expect(session()).toBeNull();
    });

    it("a tile drag whose drop never fired is reset when the safety net ends it (swallowed dragend)", () => {
        const model = fakeModel();
        startTileDrag(node, model);
        window.dispatchEvent(new Event("dragend"));
        expect(session()).toBeNull();
        expect(model.activeDrag()).toBe(false);
        expect(dragState).toMatchObject({ nodeId: null, layoutModel: null, node: null });
    });

    it("a new drag replacing a stranded tile drag resets it too", () => {
        const model = fakeModel();
        startTileDrag(node, model);
        beginDrag("window-tab", { tabId: "t2" });
        expect(model.activeDrag()).toBe(false);
        expect(dragState.layoutModel).toBeNull();
    });

    it("never ends another kind of drag", () => {
        beginDrag("pane-tab", { blockId: "b9" });
        endTileDrag("dragend");
        expect(session()?.kind).toBe("pane-tab");
        expect(isUnderway("tile")).toBe(false);
    });

    describe("the pointer leaving the window", () => {
        /** A drag event as the document gets it; `relatedTarget` null is the pointer crossing the window's edge. */
        function dragEvent(type: string, relatedTarget: EventTarget | null, dropEffect?: string) {
            const e = new Event(type, { bubbles: true }) as any;
            e.relatedTarget = relatedTarget;
            if (dropEffect) e.dataTransfer = { dropEffect };
            return e;
        }

        it("shows the tear-off when the pointer leaves the window, and takes it off when it is back", () => {
            startTileDrag(node, fakeModel());
            document.dispatchEvent(dragEvent("dragleave", null));
            document.dispatchEvent(dragEvent("dragenter", null));
            expect(hub.window).toEqual(["out:node-1", "in"]);
        });

        it("ignores a pointer that only moves from one element to another", () => {
            startTileDrag(node, fakeModel());
            document.dispatchEvent(dragEvent("dragleave", document.body));
            document.dispatchEvent(dragEvent("dragenter", document.body));
            expect(hub.window).toEqual([]);
        });

        it("watches nothing once the drag has ended with its own dragend, and one drag at a time", () => {
            startTileDrag(node, fakeModel());
            document.dispatchEvent(dragEvent("dragend", null, "move"));
            document.dispatchEvent(dragEvent("dragleave", null));
            expect(hub.window).toEqual([]);
            startTileDrag(node, fakeModel());
            startTileDrag(node, fakeModel());
            document.dispatchEvent(dragEvent("dragleave", null));
            expect(hub.window).toEqual(["out:node-1"]);
        });

        it("takes the tear-off off when the drag ended in Escape, and keeps it for a release", async () => {
            startTileDrag(node, fakeModel());
            document.dispatchEvent(dragEvent("dragend", null, "none"));
            await Promise.resolve();
            await Promise.resolve();
            expect(hub.window).toEqual([]);
            hub.escaped = true;
            startTileDrag(node, fakeModel());
            document.dispatchEvent(dragEvent("dragend", null, "none"));
            await Promise.resolve();
            await Promise.resolve();
            expect(hub.window).toEqual(["in"]);
        });
    });
});
