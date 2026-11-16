// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const watch = vi.hoisted(() => vi.fn());
const layout = vi.hoisted(() => ({ model: undefined as unknown }));
vi.mock("./escape-watch", () => ({ watchEscapeDuringDrag: watch }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: () => layout.model }));
vi.mock("@/layout/lib/homeLayout", () => ({
    homeLayoutOf: () => ({ body: () => undefined, holdsPane: () => true, active: () => true }),
}));

import type { LayoutModel } from "@/layout/lib/layoutModel";
import { setPillDrag, type PillDrag } from "@/layout/lib/pillDragState";
import { endTabTearOff, tabTearOffOffered, tabTearOffShown } from "@/layout/lib/tabTearOff";
import { endDrag } from "./drag-session";
import { startPaneTabDrag } from "./pane-tab-drag";

afterEach(() => {
    endDrag("cancel");
    endTabTearOff();
    setPillDrag(undefined);
    layout.model = undefined;
    watch.mockClear();
});

describe("a pane-tab drag's live follow", () => {
    it("follows a single tab that can tear off", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1");
        expect(watch).toHaveBeenCalledWith({ blockId: "b1" });
    });

    it("only watches Escape when several tabs are dragged together", () => {
        startPaneTabDrag("b1", "pane-1", "tab-1", ["b1", "b2"]);
        expect(watch).toHaveBeenCalledWith(undefined);
    });

    it("only watches Escape when the strip names no window tab", () => {
        startPaneTabDrag("b1", "pane-1");
        expect(watch).toHaveBeenCalledWith(undefined);
    });
});

describe("the picture in hand of a tab dragged out of a pane that keeps others", () => {
    /** A layout of one pane with two tabs, `b1` in front. */
    const twoTabs = () =>
        ({ treeState: { rootNode: { id: "pane-1", data: { blockId: "b1", activeBlockId: "b1", blockStack: ["b1", "b2"] } } } }) as unknown as LayoutModel;
    const holdsPane = () => (watch.mock.calls[0][0] as { home: { holdsPane: () => boolean } }).home.holdsPane();

    it("is carried over the tab's own place while that offers the tear-off, and from then on wherever no drop is shown", () => {
        const model = twoTabs();
        layout.model = model;
        startPaneTabDrag("b1", "pane-1", "tab-1");
        // The place is the red one, with nothing of the tab in it: the hand has the picture.
        expect(tabTearOffOffered("b1")).toBe(true);
        expect(holdsPane()).toBe(false);
        // The tab has left its pane and no drop is shown (over a header, over its own pane).
        tabTearOffShown();
        expect(holdsPane()).toBe(false);
        // A drop is shown: the tab stands in its landing place, so the hand carries nothing.
        setPillDrag({ model, blockId: "b1" } as PillDrag);
        expect(holdsPane()).toBe(true);
        setPillDrag(undefined);
        expect(holdsPane()).toBe(false);
    });

    it("is the pane's own for a pane's only tab: the layout shows that pane wherever it is", () => {
        layout.model = { treeState: { rootNode: { id: "pane-1", data: { blockId: "b1" } } } } as unknown as LayoutModel;
        startPaneTabDrag("b1", "pane-1", "tab-1");
        expect(tabTearOffOffered("b1")).toBe(false);
        expect(holdsPane()).toBe(true);
    });
});
