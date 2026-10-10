// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneTabStrip with `docDrag`: document tabs (an Editor's or a Media pane's
 * files) drag with their own kind, reorder within the strip, accept a tab
 * from another pane of the same type only when the strip takes one, and
 * never tear off. docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md.
 *
 * Its own file because it mocks pragmatic-dnd's adapter, as
 * PaneTabStrip.dropTarget.test.tsx does, to see what each pill registers.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dropTargetCalls: any[] = [];
const draggableCalls: any[] = [];
const payloadCalls: unknown[] = [];

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: (config: any) => {
        draggableCalls.push(config);
        return () => {};
    },
    dropTargetForElements: (config: any) => {
        dropTargetCalls.push(config);
        return () => {};
    },
}));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: (p: unknown) => payloadCalls.push(p),
}));

import { PaneTabStrip, type DocTabDrag } from "./PaneTabStrip";
import { docTabItemType, paneTabItemType } from "@/app/drag/drag-types";
import { endDrag, session } from "@/app/drag/drag-session";

afterEach(() => cleanup());
beforeEach(() => {
    endDrag("cancel");
    dropTargetCalls.length = 0;
    draggableCalls.length = 0;
    payloadCalls.length = 0;
});

interface T {
    id: string;
}
const TABS: T[] = [{ id: "a" }, { id: "b" }];

function renderStrip(doc: Partial<DocTabDrag> = {}) {
    const docDrag: DocTabDrag = { docType: "editor", blockId: "block-1", onReorder: vi.fn(), ...doc };
    const result = render(() => (
        <PaneTabStrip tabs={TABS} activeId="a" getId={(t: T) => t.id} getLabel={(t: T) => t.id} onActivate={vi.fn()} docDrag={docDrag} />
    ));
    const pills = [...result.container.querySelectorAll<HTMLElement>(".pane-tab")];
    const targetOf = (pill: HTMLElement) => dropTargetCalls.find((c) => c.element === pill)!;
    return { ...result, docDrag, pills, targetOf };
}

const docDrag = (tabId: string, sourceBlockId = "block-1", docType = "editor") => ({
    source: { data: { type: docTabItemType, tabId, docType, sourceBlockId } },
});
const at = (clientX: number) => ({ current: { input: { clientX } } });

describe("PaneTabStrip — document-tab drag", () => {
    it("each pill drags as a document tab of its block", () => {
        renderStrip();
        expect(draggableCalls).toHaveLength(2);
        expect(draggableCalls[1].getInitialData()).toEqual({ type: docTabItemType, tabId: "b", docType: "editor", sourceBlockId: "block-1" });
    });

    it("opens a doc-tab drag session, never a pane-tab one, and sets no tear-off payload", () => {
        const { pills } = renderStrip();
        draggableCalls[1].onDragStart();
        expect(session()?.kind).toBe("doc-tab");
        expect(session()?.source).toMatchObject({ blockId: "block-1", itemId: "b" });
        expect(pills[1].classList.contains("pane-tab--dragging")).toBe(true);
        draggableCalls[1].onDrop();
        expect(pills[1].classList.contains("pane-tab--dragging")).toBe(false);
        expect(payloadCalls).toEqual([]);
    });

    it("asks canDrag per tab", () => {
        renderStrip({ canDrag: (id) => id !== "a" });
        expect(draggableCalls[0].canDrag()).toBe(false);
        expect(draggableCalls[1].canDrag()).toBe(true);
    });

    it("accepts its own other tabs, and nothing of another kind or type", () => {
        const { pills, targetOf } = renderStrip();
        const onA = targetOf(pills[0]);
        expect(onA.canDrop(docDrag("b"))).toBe(true);
        expect(onA.canDrop(docDrag("a"))).toBe(false); // onto itself
        expect(onA.canDrop(docDrag("x", "block-1", "media"))).toBe(false); // another pane type
        expect(onA.canDrop({ source: { data: { type: paneTabItemType, blockId: "b", sourceNodeId: "pane" } } })).toBe(false);
    });

    it("a tab from another pane is accepted only when the strip takes one", () => {
        expect(renderStrip().targetOf(document.querySelectorAll<HTMLElement>(".pane-tab")[0] as HTMLElement).canDrop(docDrag("z", "block-2"))).toBe(false);
        cleanup();
        dropTargetCalls.length = 0;
        const { pills, targetOf } = renderStrip({ onReceive: vi.fn() });
        expect(targetOf(pills[0]).canDrop(docDrag("z", "block-2"))).toBe(true);
    });

    it("a drop from its own strip reorders, at the side under the pointer", () => {
        const { pills, targetOf, docDrag: doc } = renderStrip();
        targetOf(pills[0]).onDrop({ ...docDrag("b"), location: at(-1) });
        expect(doc.onReorder).toHaveBeenCalledWith("b", "a", "before");
        expect(payloadCalls).toEqual([]); // nothing to tear off, so nothing to clear
    });

    it("a drop from another pane is received one task later, at the tab it landed on", async () => {
        const onReceive = vi.fn();
        const { pills, targetOf } = renderStrip({ onReceive });
        targetOf(pills[1]).onDrop({ ...docDrag("z", "block-2"), location: at(1) });
        expect(onReceive).not.toHaveBeenCalled();
        await new Promise((r) => setTimeout(r, 0));
        expect(onReceive).toHaveBeenCalledWith("block-2", "z", { targetId: "b", position: "after" });
    });
});
