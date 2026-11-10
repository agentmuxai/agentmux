// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { holdBottomEdge } from "./hold-bottom-edge";
import { computeMenuPosition } from "./menu-position";

const VIEWPORT_W = 1200;
const VIEWPORT_H = 800;

function makeRect(x: number, y: number, w: number, h: number): DOMRect {
    return { x, y, width: w, height: h, top: y, left: x, right: x + w, bottom: y + h, toJSON: () => ({}) } as DOMRect;
}

/** A detached element of a fixed size: floating-ui reads `offsetWidth`/`offsetHeight`, jsdom reports 0 for both. */
function elementWithRect(rect: DOMRect): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () => rect;
    Object.defineProperty(el, "offsetWidth", { value: rect.width, configurable: true });
    Object.defineProperty(el, "offsetHeight", { value: rect.height, configurable: true });
    return el;
}

beforeEach(() => {
    window.innerWidth = VIEWPORT_W;
    window.innerHeight = VIEWPORT_H;
    Object.defineProperty(document.documentElement, "clientWidth", { value: VIEWPORT_W, configurable: true });
    Object.defineProperty(document.documentElement, "clientHeight", { value: VIEWPORT_H, configurable: true });
});

afterEach(() => {
    document.body.innerHTML = "";
});

describe("holdBottomEdge", () => {
    /** The rect a panel of `height` takes under `style` in the test viewport. */
    const rectUnder = (style: { top?: unknown; bottom?: unknown }, height: number) => {
        const top = style.bottom !== undefined ? VIEWPORT_H - Number.parseFloat(String(style.bottom)) - height : Number.parseFloat(String(style.top));
        return { top, bottom: top + height };
    };

    it("pins a panel placed above its anchor by its bottom edge, so it grows away from the anchor", async () => {
        // The readout's case: an anchor 15 px tall at the window's bottom edge, a panel that opens at 114 px
        // and grows to 180 when its detail rows arrive.
        const anchor = makeRect(900, VIEWPORT_H - 16, 64, 15);
        const placed = await computeMenuPosition({ anchor, placement: "bottom", gutter: 10, avoidNativePanes: false }, elementWithRect(makeRect(0, 0, 458, 114)));
        expect(placed.placement).toBe("top");
        const style = holdBottomEdge(placed, 114, VIEWPORT_H);
        for (const height of [114, 180, 400]) {
            const rect = rectUnder(style, height);
            expect(rect.bottom, `a ${height} px panel`).toBeLessThanOrEqual(anchor.top - 10);
        }
        // Pinned by the top edge, the same growth lands on the anchor: this is the loop it ends.
        expect(rectUnder(placed.style, 180).bottom).toBeGreaterThan(anchor.top);
    });

    it("leaves a panel on any other side as it was placed", async () => {
        const anchor = makeRect(100, 100, 64, 15);
        const placed = await computeMenuPosition({ anchor, placement: "bottom", gutter: 10, avoidNativePanes: false }, elementWithRect(makeRect(0, 0, 458, 114)));
        expect(placed.placement).toBe("bottom");
        expect(holdBottomEdge(placed, 114, VIEWPORT_H)).toBe(placed.style);
    });
});
