// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { paneShapeOf, settledLine } from "./pane-cover-log";

function scroller(top: number, scrollHeight: number, clientHeight: number): HTMLElement {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollHeight", { value: scrollHeight });
    Object.defineProperty(el, "clientHeight", { value: clientHeight });
    Object.defineProperty(el, "scrollTop", { value: top });
    return el;
}

describe("paneShapeOf", () => {
    it("says so when the pane's element is not found", () => {
        expect(paneShapeOf(null)).toBe("root=missing");
    });

    it("counts the elements and reports the tallest scroller's position", () => {
        const root = document.createElement("div");
        root.append(scroller(0, 400, 300), scroller(120, 9000, 600), document.createElement("span"));
        expect(paneShapeOf(root)).toBe("size=0x0 nodes=3 scroller(top/height/view)=120/9000/600");
    });

    it("reports no scroller when nothing overflows", () => {
        const root = document.createElement("div");
        root.append(scroller(0, 300, 300));
        expect(paneShapeOf(root)).toBe("size=0x0 nodes=1 scroller(top/height/view)=none");
    });

    it("puts the shape in the settled line", () => {
        expect(settledLine("0123456789", 3499.6, document.createElement("div"), "Manoz")).toBe(
            'block=0123456 agent="Manoz" settled after-cover=3500ms size=0x0 nodes=0 scroller(top/height/view)=none'
        );
    });
});
