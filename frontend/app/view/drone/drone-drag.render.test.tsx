// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dragging a node chip from the top bar onto the canvas, and the canvas
 * leaving every other drag alone. SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md
 * §5.3 (the canvas type check) and §5.8 (characterisation before phase 4f).
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

function mount() {
    model = new DroneViewModel({ blockId: "drone-drag-block", meta: () => ({}) } as any);
    const result = render(() => <DroneView model={model!} />);
    const canvas = result.container.querySelector<HTMLElement>(".drone-canvas")!;
    const chip = result.container.querySelector<HTMLElement>(".drone-chip")!;
    const nodeCount = () => result.container.querySelectorAll(".drone-node").length;
    return { canvas, chip, nodeCount };
}

/** A drag event carrying a minimal DataTransfer (jsdom has none). */
function dragEvent(type: string, types: string[] = [], data: Record<string, string> = {}): Event {
    const e = new Event(type, { bubbles: true, cancelable: true });
    const store: Record<string, string> = { ...data };
    Object.defineProperty(e, "dataTransfer", {
        value: {
            types,
            dropEffect: "none",
            effectAllowed: "all",
            setData: (k: string, v: string) => (store[k] = v),
            getData: (k: string) => store[k] ?? "",
        },
    });
    Object.assign(e, { clientX: 100, clientY: 100 });
    return e;
}

describe("drone chip → canvas", () => {
    it("a chip drag is accepted over the canvas and drops a node there", () => {
        const { canvas, chip, nodeCount } = mount();
        chip.dispatchEvent(dragEvent("dragstart"));
        const over = dragEvent("dragover");
        canvas.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(true);
        expect((over as any).dataTransfer.dropEffect).toBe("copy");
        canvas.dispatchEvent(dragEvent("drop"));
        expect(nodeCount()).toBe(1);
        chip.dispatchEvent(dragEvent("dragend"));
    });

    it("an OS file drag is left alone: no copy cursor, nothing dropped", () => {
        const { canvas, nodeCount } = mount();
        const over = dragEvent("dragover", ["Files"]);
        canvas.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(false);
        canvas.dispatchEvent(dragEvent("drop", ["Files"]));
        expect(nodeCount()).toBe(0);
    });

    it("once the chip's drag has ended, the canvas stops accepting", () => {
        const { canvas, chip } = mount();
        chip.dispatchEvent(dragEvent("dragstart"));
        chip.dispatchEvent(dragEvent("dragend"));
        const over = dragEvent("dragover");
        canvas.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(false);
    });

    it("a chip dragged from another window still drops, by its MIME type", () => {
        const { canvas, nodeCount } = mount();
        const types = ["application/x-drone-kind"];
        const over = dragEvent("dragover", types);
        canvas.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(true);
        canvas.dispatchEvent(dragEvent("drop", types, { "application/x-drone-kind": "agent" }));
        expect(nodeCount()).toBe(1);
    });
});
