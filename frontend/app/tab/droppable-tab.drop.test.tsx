// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a drop on a window tab's button takes along: a whole pane moves with every one of its tabs, a pane-tab
 * pill with just its own tab (or the picked ones), and both go through the one cross-tab move.
 * docs/specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    targets: [] as any[],
    moves: [] as any[],
    payload: null as any,
    selected: 0,
}));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: () => () => {},
    dropTargetForElements: (args: any) => {
        hub.targets.push(args);
        return () => {};
    },
}));
vi.mock("@atlaskit/pragmatic-drag-and-drop/prevent-unhandled", () => ({ preventUnhandled: { start() {}, stop() {} } }));
vi.mock("@/layout/index", () => ({
    getLayoutModelForTabById: () => undefined,
    moveBlocksToTab: (move: any) => hub.moves.push(move),
}));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    getCurrentDragPayload: () => hub.payload,
    setCurrentDragPayload: (p: any) => (hub.payload = p),
}));
vi.mock("@/store/global", () => ({ getApi: () => ({}) }));
vi.mock("./tab", () => ({ Tab: () => <div class="tab" /> }));
vi.mock("@/app/view/browser/hibernation", () => ({ hoverTabPill() {}, leaveTabPill() {}, pressTabPill() {} }));
vi.mock("@/app/drag/tearoff-snapshot", () => ({ prewarmWindowTabSnapshot() {} }));
vi.mock("@/app/drag/wayland-drag-image", () => ({ hideDragImageOnNativeWayland() {} }));
vi.mock("@/app/motion/solid", () => ({ usePulseWhen() {} }));

import { paneTabItemType, tileItemType } from "@/app/drag/drag-types";
import { DroppableTab } from "./droppable-tab";

function mount(tabId = "tab-b") {
    render(() => (
        <DroppableTab
            tabId={tabId}
            workspaceId="ws-1"
            activeTabId="tab-a"
            isActive={false}
            isFirst={false}
            isBeforeActive={false}
            allTabCount={3}
            tabIndex={1}
            tabIds={["tab-a", "tab-b", "tab-c"]}
            onSelect={() => {}}
            onClose={() => {}}
        />
    ));
    // The window tab's own drop target is the one that takes panes and pills.
    return hub.targets.find((t) => t.canDrop?.({ source: { data: { type: tileItemType } } }))!;
}

const pill = (data: Record<string, unknown>) => ({ source: { data: { type: paneTabItemType, ...data } } });

beforeEach(() => {
    hub.targets = [];
    hub.moves = [];
    hub.payload = null;
});
afterEach(cleanup);

describe("a drop on a window tab's button", () => {
    it("takes pane-tab pills as well as panes, and nothing else", () => {
        const target = mount();
        expect(target.canDrop({ source: { data: { type: paneTabItemType } } })).toBe(true);
        expect(target.canDrop({ source: { data: { type: tileItemType } } })).toBe(true);
        expect(target.canDrop({ source: { data: { type: "TAB_ITEM" } } })).toBe(false);
    });

    it("moves a whole pane with all of its tabs, the visible one kept", () => {
        const target = mount();
        hub.payload = {
            kind: "tile",
            sourceTabId: "tab-a",
            node: { data: { blockId: "b", blockStack: ["a", "b", "c"], activeBlockId: "b" } },
        };
        target.onDrop({ source: { data: { type: tileItemType } } });
        expect(hub.moves).toEqual([{ sourceTabId: "tab-a", blockIds: ["a", "b", "c"], activeBlockId: "b", targetTabId: "tab-b" }]);
        expect(hub.payload).toBeNull();
    });

    it("moves a pane with one tab as that one tab", () => {
        const target = mount();
        hub.payload = { kind: "tile", sourceTabId: "tab-a", node: { data: { blockId: "solo" } } };
        target.onDrop({ source: { data: { type: tileItemType } } });
        expect(hub.moves).toEqual([{ sourceTabId: "tab-a", blockIds: ["solo"], activeBlockId: "solo", targetTabId: "tab-b" }]);
    });

    it("moves just the pill's own tab, not the rest of its pane", () => {
        const target = mount();
        target.onDrop(pill({ blockId: "b", sourceTabId: "tab-a", sourceNodeId: "n1" }));
        expect(hub.moves).toEqual([{ sourceTabId: "tab-a", blockIds: ["b"], activeBlockId: "b", targetTabId: "tab-b" }]);
    });

    it("moves the picked tabs together when several pills were dragged", () => {
        const target = mount();
        target.onDrop(pill({ blockId: "c", blockIds: ["a", "c"], sourceTabId: "tab-a" }));
        expect(hub.moves).toEqual([{ sourceTabId: "tab-a", blockIds: ["a", "c"], activeBlockId: "c", targetTabId: "tab-b" }]);
    });

    it("does nothing for a drop on the tab the tabs are already in", () => {
        const target = mount("tab-a");
        target.onDrop(pill({ blockId: "b", sourceTabId: "tab-a" }));
        hub.payload = { kind: "tile", sourceTabId: "tab-a", node: { data: { blockId: "b" } } };
        target.onDrop({ source: { data: { type: tileItemType } } });
        expect(hub.moves).toEqual([]);
    });

    it("does nothing for a pill that does not say which tab it came from", () => {
        const target = mount();
        target.onDrop(pill({ blockId: "b" }));
        expect(hub.moves).toEqual([]);
    });
});
