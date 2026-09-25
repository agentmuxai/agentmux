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

import { Placeholder } from "../lib/tilelayout-shared";

describe("Placeholder (the drag drop-target ghost)", () => {
    it("takes part in pane-overlay clipping on the element that moves", () => {
        const [xf, setXf] = createSignal<Record<string, string> | null>(null);
        const { container } = render(() => (
            <Placeholder layoutModel={{ placeholderTransform: xf } as any} style={{}} />
        ));
        expect(container.querySelector(".placeholder-sizer")).toBeNull();

        setXf({ transform: "translate(10px, 20px)", width: "300px", height: "200px" });
        const sizer = container.querySelector(".placeholder-sizer") as HTMLElement;
        expect(sizer).not.toBeNull();
        expect(sizer.hasAttribute("data-pane-overlay")).toBe(true);
        expect(sizer.style.transform).toBe("translate(10px, 20px)");
    });
});
