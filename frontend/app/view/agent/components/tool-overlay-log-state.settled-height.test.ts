// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { settledLogHeight } from "./tool-overlay-log-state";

/** A box with the three heights the browser reports (there is no layout here). */
function box(scrollHeight: number, clientHeight: number, offsetHeight: number): HTMLElement {
    return { scrollHeight, clientHeight, offsetHeight } as HTMLElement;
}

describe("settledLogHeight", () => {
    it("is the content height when there is no horizontal scrollbar", () => {
        expect(settledLogHeight(box(35, 35, 35))).toBe(35);
    });

    it("adds the horizontal scrollbar a long line brings, so the ease ends where the box lands", () => {
        // A one-line result wider than the pane: 35 px of content, a 7 px scrollbar under it.
        expect(settledLogHeight(box(35, 35, 42))).toBe(42);
    });

    it("keeps counting content past the cap, plus the scrollbar", () => {
        // Content taller than the panel's cap: the box is clamped, scrollHeight is not.
        expect(settledLogHeight(box(900, 189, 196))).toBe(907);
    });

    it("counts no scrollbar for a box that is not laid out", () => {
        expect(settledLogHeight(box(35, 0, 50))).toBe(35);
    });

    it("never goes below the content height", () => {
        expect(settledLogHeight(box(35, 40, 35))).toBe(35);
    });
});
