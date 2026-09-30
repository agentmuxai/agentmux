// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tab bar's end-of-drag cleanup for whole-pane (tile) drags. A stuck
 * `activeDrag` is a dead window tab: its overlay covers the panes and eats
 * every click. Characterises today's behaviour before the safety nets merge
 * (SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1, §5.8).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    monitors: [] as any[],
    models: new Map<string, { activeDrag: any }>(),
    pruned: [] as string[],
}));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    monitorForElements: (args: any) => {
        hub.monitors.push(args);
        return () => {};
    },
    dropTargetForElements: () => () => {},
}));
vi.mock("@/layout/index", () => ({
    clearCrossTabDrop: () => {},
    getLayoutModelForTabById: (id: string) => hub.models.get(id),
}));
vi.mock("@/layout/lib/layoutPersistence", () => ({
    pruneDanglingLeaves: (m: any) => hub.pruned.push(m.id),
}));
vi.mock("@/layout/lib/crossTabDrag", () => ({ clearCrossTabDrop: () => {} }));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({ setCurrentDragPayload: () => {} }));
vi.mock("../store/services", () => ({ WorkspaceService: { ReorderTab: () => Promise.resolve() } }));
vi.mock("@/util/platformutil", async (importOriginal) => ({
    ...(await importOriginal<Record<string, unknown>>()),
    isWindows: () => true,
}));

import { beginDrag, endDrag, markReleased } from "@/app/drag/drag-session";
import { tileItemType } from "@/app/drag/drag-types";
import { dragActivatedTabIds, startWindowTabDrag } from "./tabbar-dnd";
import { useTabDragAndDrop } from "./tab-reorder";

function fakeModel(id: string) {
    const [get, set] = createSignal(false);
    const model = { id, activeDrag: Object.assign(get, { _set: set }) };
    hub.models.set(id, model);
    return model;
}

/** The tab strip's rect: x 0-500, y 0-30. */
const STRIP_RECT = { left: 0, right: 500, top: 0, bottom: 30, x: 0, y: 0, width: 500, height: 30 };

function mount() {
    const tabIds = ["tab-1", "tab-2"];
    const models = tabIds.map(fakeModel);
    render(() => {
        let strip!: HTMLDivElement;
        queueMicrotask(() => (strip.getBoundingClientRect = () => STRIP_RECT as DOMRect));
        useTabDragAndDrop(
            { tabBarScrollRef: () => strip },
            () => ({ oid: "ws-1" }) as any,
            () => tabIds,
            vi.fn()
        );
        return <div ref={strip} />;
    });
    return models;
}

/** A pane drag from tab-1 spring-switched through tab-2: its overlay was forced on. */
function springSwitchedThrough(model: { activeDrag: any }, tabId: string) {
    beginDrag("tile", { nodeId: "n1", tabId: "tab-1" });
    model.activeDrag._set(true);
    dragActivatedTabIds.add(tabId);
}

beforeEach(() => {
    hub.monitors = [];
    hub.models.clear();
    dragActivatedTabIds.clear();
});
afterEach(() => {
    cleanup();
    endDrag("cancel");
});

describe("tab bar end-of-drag cleanup (tile drags)", () => {
    it("a window dragend resets every window tab's overlay after a spring switch", () => {
        const [, tab2] = mount();
        springSwitchedThrough(tab2, "tab-2");
        window.dispatchEvent(new Event("dragend"));
        expect(tab2.activeDrag()).toBe(false);
        expect(dragActivatedTabIds.size).toBe(0);
    });

    it("a pointerdown with spring-switched tabs still recorded resets them: the drag ended unobserved", () => {
        const [, tab2] = mount();
        springSwitchedThrough(tab2, "tab-2");
        window.dispatchEvent(new Event("pointerdown"));
        expect(tab2.activeDrag()).toBe(false);
        expect(dragActivatedTabIds.size).toBe(0);
    });

    it("the tile monitor's drop resets the overlays and prunes each tab once the drag settles", () => {
        vi.useFakeTimers();
        try {
            const [tab1, tab2] = mount();
            tab1.activeDrag._set(true);
            springSwitchedThrough(tab2, "tab-2");
            const tileMonitor = hub.monitors.find((m) => m.canMonitor?.({ source: { data: { type: tileItemType } } }));
            expect(tileMonitor).toBeDefined();
            tileMonitor.onDrop({ source: { data: { type: tileItemType } }, location: { current: { dropTargets: [] } } });
            expect(tab1.activeDrag()).toBe(false);
            expect(tab2.activeDrag()).toBe(false);
            vi.advanceTimersByTime(250);
            expect(hub.pruned).toEqual(["tab-1", "tab-2"]);
        } finally {
            vi.useRealTimers();
        }
    });
});

/** A window dragover at (x, y), with a DataTransfer to read the cursor off. */
function dragOverAt(clientX: number, clientY: number) {
    const e = new Event("dragover", { bubbles: true, cancelable: true });
    Object.assign(e, { clientX, clientY });
    Object.defineProperty(e, "dataTransfer", { value: { dropEffect: "none" } });
    document.body.dispatchEvent(e);
    return e as Event & { dataTransfer: { dropEffect: string } };
}

describe("tab bar tear-off cursor (Windows)", () => {
    it("below the strip during a window-tab drag: the plus cursor, since releasing there tears off", async () => {
        mount();
        await Promise.resolve();
        startWindowTabDrag("tab-1", "ws-1", true);
        const e = dragOverAt(200, 200);
        expect(e.defaultPrevented).toBe(true);
        expect(e.dataTransfer.dropEffect).toBe("copy");
    });

    it("over the strip: left to the strip's own drop target", async () => {
        mount();
        await Promise.resolve();
        startWindowTabDrag("tab-1", "ws-1", true);
        const e = dragOverAt(200, 15);
        expect(e.defaultPrevented).toBe(false);
    });

    it("not for other drags, or once the tab's source has released it", async () => {
        mount();
        await Promise.resolve();
        beginDrag("tile", { nodeId: "n1" });
        expect(dragOverAt(200, 200).defaultPrevented).toBe(false);
        startWindowTabDrag("tab-1", "ws-1", true);
        markReleased();
        expect(dragOverAt(200, 200).defaultPrevented).toBe(false);
    });
});
