// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const layout = vi.hoisted(() => ({
    models: new Map<string, { loaded: boolean; blockIds: string[] }>(),
}));
vi.mock("@/layout/index", () => ({
    peekLayoutModelForTab: (tabId: string) => {
        const m = layout.models.get(tabId);
        if (!m) return undefined;
        return {
            muxObjectAtom: () => (m.loaded ? { oid: "layout" } : undefined),
            getter: (accessor: () => unknown) => accessor(),
            leafs: () => m.blockIds.map((blockId) => ({ data: { blockId } })),
        };
    },
}));

import { holdPaneContent, registerPaneMounted, trackPaneContent } from "@/app/store/pane-content-holds";
import { tabWasShown } from "@/app/store/tab-reveal";
import { markLoadedTabsShownWhenSettled, tabContentSettled, whenTabContentSettled } from "./tab-content-settled";

// A pane as a reporting view mounts it: mounted and tracked.
const mountTracked = (blockId: string) => {
    const unmount = registerPaneMounted(blockId);
    const untrack = trackPaneContent(blockId);
    return () => {
        unmount();
        untrack();
    };
};

describe("tab content settled", () => {
    const cleanups: (() => void)[] = [];
    beforeEach(() => layout.models.clear());
    afterEach(() => {
        cleanups.splice(0).forEach((f) => f());
    });

    it("is not settled before the tab has a layout model, or before its layout loaded", () => {
        expect(tabContentSettled("t0")).toBe(false);
        layout.models.set("t0", { loaded: false, blockIds: [] });
        expect(tabContentSettled("t0")).toBe(false);
        layout.models.get("t0")!.loaded = true;
        expect(tabContentSettled("t0")).toBe(true);
    });

    it("waits for every pane to mount and release its holds", () => {
        layout.models.set("t1", { loaded: true, blockIds: ["a", "b"] });
        cleanups.push(mountTracked("a"));
        expect(tabContentSettled("t1")).toBe(false);
        cleanups.push(mountTracked("b"));
        const hold = holdPaneContent("b");
        expect(tabContentSettled("t1")).toBe(false);
        hold();
        expect(tabContentSettled("t1")).toBe(true);
    });

    it("whenTabContentSettled resolves once settled, or false at the cap", async () => {
        layout.models.set("t2", { loaded: true, blockIds: ["c"] });
        const settled = whenTabContentSettled("t2", 1000);
        cleanups.push(mountTracked("c"));
        await expect(settled).resolves.toBe(true);
        await expect(whenTabContentSettled("never", 30)).resolves.toBe(false);
    });

    it("marks each loaded tab shown as it settles", async () => {
        layout.models.set("t3", { loaded: true, blockIds: ["d"] });
        layout.models.set("t4", { loaded: true, blockIds: ["e"] });
        cleanups.push(mountTracked("d"));
        cleanups.push(markLoadedTabsShownWhenSettled(["t3", "t4"], () => true));
        await vi.waitFor(() => expect(tabWasShown("t3")).toBe(true));
        expect(tabWasShown("t4")).toBe(false);
        cleanups.push(mountTracked("e"));
        await vi.waitFor(() => expect(tabWasShown("t4")).toBe(true));
    });

    it("never settles a pane whose view doesn't report its loading (Codex on #4132)", () => {
        layout.models.set("t6", { loaded: true, blockIds: ["untracked"] });
        cleanups.push(registerPaneMounted("untracked"));
        expect(tabContentSettled("t6")).toBe(false);
    });

    it("marks nothing while inactive tabs aren't kept laid out", async () => {
        layout.models.set("t5", { loaded: true, blockIds: [] });
        cleanups.push(markLoadedTabsShownWhenSettled(["t5"], () => false));
        await new Promise((r) => setTimeout(r, 50));
        expect(tabWasShown("t5")).toBe(false);
    });
});
