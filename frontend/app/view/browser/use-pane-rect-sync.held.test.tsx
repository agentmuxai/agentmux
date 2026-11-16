// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A browser page is held where it is for the length of an in-app drag: it is cut away behind a picture, and
 * the live reflow draws its pane's tile wherever the drop would put it (liveReflow.ts). The page must not
 * follow the tile (it would lay itself out again for every drop the drag passes over), and it must be exactly
 * where its hole is. Hiding and showing still go through, and the real rect goes out when the hold ends.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const sent: { x: number; y: number; width: number; height: number }[] = [];
vi.mock("./pane-rect-batcher", () => ({
    paneRectBatcher: () => ({ set: (_id: string, rect: (typeof sent)[number]) => sent.push({ ...rect }), forget: () => {}, flushNow: () => {} }),
}));
vi.mock("./pane-resize-observer", () => ({ observePaneElement: () => () => {} }));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({ browserPanes: { create: async () => {}, close: async () => {} } }),
}));
vi.mock("@/app/workspace/floater-resize", () => ({ FLOATER_BROWSER_EDGE_INSET: 4 }));
vi.mock("@/app/platform/pane-backend", () => ({ isViewsPanesOnWindows: () => false }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { claimFocusOnMount: () => {} } }));
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: () => visibility }));
vi.mock("@/app/platform/pane-rect-registry", () => ({ registerPaneRect: () => {}, unregisterPaneRect: () => {} }));
vi.mock("@/app/platform/pane-anim", () => ({ paneReflowActive: () => false, notifyPaneReflow: () => {} }));

import { usePaneRectSync, type PaneRectSync } from "./use-pane-rect-sync";

const model = {
    blockId: "held1",
    setPageLive: () => {},
    closed: false,
    urlAtom: () => "https://agentmux.ai/",
    driverAgentAtom: () => undefined,
    allowedOriginsAtom: () => [],
    identityAtom: () => undefined,
} as any;

const [held, setHeld] = createSignal(false);
let box = { x: 10, y: 20, width: 300, height: 200 };
let sync: PaneRectSync;

function Mount() {
    let ph: HTMLDivElement | undefined;
    sync = usePaneRectSync({ model, placeholderRef: () => ph, windowLabel: "main", diag: () => {}, held });
    return <div ref={ph} />;
}

// Real timers: the hook settles on its own.
const tick = (ms = 30) => new Promise((r) => setTimeout(r, ms));

async function mounted() {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => box as DOMRect);
    render(() => <Mount />);
    await tick();
    sync.syncPosition();
    expect(sent.at(-1)).toEqual({ x: 10, y: 20, width: 300, height: 200 });
    sent.length = 0;
}

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    sent.length = 0;
    setHeld(false);
    setVisibility("active");
    box = { x: 10, y: 20, width: 300, height: 200 };
});

describe("usePaneRectSync: a page held for a drag", () => {
    it("stays where it is while its placeholder is drawn elsewhere, and goes to it when the hold ends", async () => {
        await mounted();
        setHeld(true);
        // The live reflow moves and resizes the tile.
        box = { x: 400, y: 20, width: 500, height: 300 };
        sync.syncPosition();
        await tick(250); // the safety-net interval too
        expect(sent).toEqual([]);
        expect(sync.pageRectCss()).toEqual({ x: 10, y: 20, w: 300, h: 200 });
        setHeld(false);
        await tick();
        expect(sent.at(-1)).toEqual({ x: 400, y: 20, width: 500, height: 300 });
        expect(sync.pageRectCss()).toEqual({ x: 400, y: 20, w: 500, h: 300 });
    });

    it("still collapses when its tab is hidden under the drag, and is laid out when it is shown again", async () => {
        await mounted();
        setHeld(true);
        setVisibility("dormant");
        await tick();
        expect(sent.at(-1)).toEqual({ x: 0, y: 0, width: 0, height: 0 });
        expect(sync.pageRectCss()).toBeUndefined();
        box = { x: 40, y: 50, width: 320, height: 240 };
        setVisibility("active");
        await tick();
        // Shown again mid-drag: it takes its place once, and is held there.
        expect(sent.at(-1)).toEqual({ x: 40, y: 50, width: 320, height: 240 });
        expect(sync.pageRectCss()).toEqual({ x: 40, y: 50, w: 320, h: 240 });
        sent.length = 0;
        box = { x: 90, y: 50, width: 320, height: 240 };
        sync.syncPosition();
        expect(sent).toEqual([]);
    });

    it("moves as before when nothing holds it", async () => {
        await mounted();
        box = { x: 400, y: 20, width: 500, height: 300 };
        sync.syncPosition();
        expect(sent).toEqual([{ x: 400, y: 20, width: 500, height: 300 }]);
    });
});
