// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A whole-pane (tile) drag's shared state, from start to release to end.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1, §5.8.
 */

import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ payloads: [] as unknown[], crossTabCleared: 0 }));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: (p: unknown) => hub.payloads.push(p),
}));
vi.mock("./crossTabDrag", () => ({ clearCrossTabDrop: () => hub.crossTabCleared++ }));

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
});
