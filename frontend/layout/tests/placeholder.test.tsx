// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drag drop-target ghost must show over a browser pane. A browser page is
 * a native window composited above the DOM, so a DOM element is hidden under
 * it unless it takes part in pane-overlay clipping (`data-pane-overlay`,
 * pane-overlay-auto.ts), which cuts a matching hole through the page. The tag
 * goes on the element that MOVES — `.placeholder-sizer` carries the inline
 * transform — since the tracker re-measures on that element's own style
 * changes; the inner `.placeholder` never changes its own style.
 */

import { render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({ setCurrentDragPayload: () => {} }));
vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({ dropTargetForElements: () => () => {} }));

import { beginPaneCapture, resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { Placeholder } from "../lib/tilelayout-shared";

const ghost = { transform: "translate(10px, 20px)", width: "300px", height: "200px" };

describe("Placeholder (the drag drop-target ghost)", () => {
    it("takes part in pane-overlay clipping on the element that moves", () => {
        const [xf, setXf] = createSignal<Record<string, string> | null>(null);
        const model = { placeholderTransform: xf, placementWireframe: () => [], liveReflowActive: () => false };
        const { container } = render(() => <Placeholder layoutModel={model as any} style={{}} />);
        expect(container.querySelector(".placeholder-sizer")).toBeNull();

        setXf(ghost);
        const sizer = container.querySelector(".placeholder-sizer") as HTMLElement;
        expect(sizer).not.toBeNull();
        expect(sizer.hasAttribute("data-pane-overlay")).toBe(true);
        expect(sizer.style.transform).toBe("translate(10px, 20px)");
    });

    it("draws the other panes' wireframe, unless the live reflow is moving the panes themselves (liveReflow.ts)", () => {
        const [reflowing, setReflowing] = createSignal(false);
        const model = {
            placeholderTransform: () => ghost,
            placementWireframe: () => [{ id: "b", style: { transform: "translate(0px, 0px)" } }],
            liveReflowActive: reflowing,
        };
        const { container } = render(() => <Placeholder layoutModel={model as any} style={{}} />);
        expect(container.querySelectorAll(".placement-wireframe").length).toBe(1);
        expect(container.querySelector(".placeholder-container")!.classList.contains("live-reflow")).toBe(false);

        setReflowing(true);
        expect(container.querySelectorAll(".placement-wireframe").length).toBe(0);
        expect(container.querySelector(".placeholder-container")!.classList.contains("live-reflow")).toBe(true);
        expect(container.querySelector(".placeholder-sizer")).not.toBeNull();
    });

    it("stays away while the dragged pane's picture is being taken: its tint would be in the picture", () => {
        resetPaneCaptureGateForTests();
        const captured = beginPaneCapture();
        const model = { placeholderTransform: () => ghost, placementWireframe: () => [], liveReflowActive: () => false };
        const { container } = render(() => <Placeholder layoutModel={model as any} style={{}} />);
        expect(container.querySelector(".placeholder-sizer")).toBeNull();
        captured();
        expect(container.querySelector(".placeholder-sizer")).not.toBeNull();
    });
});
