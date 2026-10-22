// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Drone and the shared pane zoom: it scales the bar, the canvas controls and the run panel,
 * not the graph, which keeps its own zoom; Ctrl+scroll is the pane zoom, a plain scroll the
 * graph's. REPORT_PANE_ZOOM_COVERAGE_2026_10_22.md §3.4.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        ListBundlesCommand: () => Promise.resolve([]),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { isModKey } from "@/util/platformutil";
import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

// jsdom has no ResizeObserver; the canvas only uses it to track its own size.
vi.stubGlobal(
    "ResizeObserver",
    class {
        observe() {}
        unobserve() {}
        disconnect() {}
    }
);

let model: DroneViewModel | null = null;
afterEach(() => {
    cleanup();
    model?.dispose();
    model = null;
});

function mount(meta: Record<string, unknown> = {}) {
    model = new DroneViewModel({ blockId: "drone-zoom-block", meta: () => meta } as unknown as PaneTabHostContext);
    const { container } = render(() => <DroneView model={model!} />);
    const el = (selector: string) => container.querySelector<HTMLElement>(selector)!;
    return { el, model: model! };
}

// With the modifier, both Ctrl and Cmd are held, so it is the platform's zoom modifier on any OS.
const wheel = (mod: boolean) =>
    new WheelEvent("wheel", {
        deltaY: -100,
        ctrlKey: mod,
        metaKey: mod,
        bubbles: true,
        cancelable: true,
        clientX: 10,
        clientY: 10,
    });

describe("Drone pane zoom", () => {
    it("scales the bar and the canvas controls, not the graph", () => {
        const { el } = mount({ "term:zoom": 1.5 });
        expect(el(".drone-bar").style.zoom).toBe("1.5");
        expect(el(".drone-controls").style.zoom).toBe("1.5");
        expect(el(".drone-viewport").style.zoom).toBe("");
        expect(el(".drone-canvas").style.zoom).toBe("");
    });

    it("leaves Ctrl+scroll to the pane zoom: the graph's zoom doesn't move", () => {
        const { el, model } = mount();
        const ev = wheel(true);
        el(".drone-canvas").dispatchEvent(ev);
        expect(model.viewportAtom().zoom).toBe(1);
        expect(ev.defaultPrevented).toBe(false);
    });

    it("still zooms the graph on a plain scroll", () => {
        const { el, model } = mount();
        el(".drone-canvas").dispatchEvent(wheel(false));
        expect(model.viewportAtom().zoom).toBeGreaterThan(1);
    });

    it("keeps a wheel with the other modifier (a macOS pinch is Ctrl+wheel) for the graph only", () => {
        const { el, model } = mount();
        // Whichever of Ctrl and Cmd isn't this platform's zoom modifier.
        const ctrlOnly = { ctrlKey: true, metaKey: false };
        const other = isModKey(ctrlOnly) ? { ctrlKey: false, metaKey: true } : ctrlOnly;
        const reachedWindow = vi.fn();
        window.addEventListener("wheel", reachedWindow);
        try {
            el(".drone-canvas").dispatchEvent(
                new WheelEvent("wheel", {
                    deltaY: -100,
                    ...other,
                    bubbles: true,
                    cancelable: true,
                    clientX: 10,
                    clientY: 10,
                })
            );
        } finally {
            window.removeEventListener("wheel", reachedWindow);
        }
        expect(model.viewportAtom().zoom).toBeGreaterThan(1);
        expect(reachedWindow).not.toHaveBeenCalled();
    });
});
