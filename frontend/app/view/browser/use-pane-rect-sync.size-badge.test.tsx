// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { insetForSizeBadge, SIZE_BADGE_STRIP_CSS } from "./use-pane-rect-sync";

// While a splitter is dragged the native page stops short of the pane's bottom,
// leaving the pane size badge's strip to the DOM (it would sit under the page).
describe("insetForSizeBadge", () => {
    it("takes the badge strip off the bottom, in device pixels", () => {
        const rect = { x: 10, y: 40, width: 800, height: 600 };
        expect(insetForSizeBadge(rect, 1)).toEqual({ x: 10, y: 40, width: 800, height: 600 - SIZE_BADGE_STRIP_CSS });
        expect(insetForSizeBadge(rect, 1.25)).toEqual({ x: 10, y: 40, width: 800, height: 600 - 40 });
    });

    it("clears the badge: 8px inset plus a ~21px tall badge", () => {
        expect(SIZE_BADGE_STRIP_CSS).toBeGreaterThanOrEqual(30);
    });

    it("never collapses a tiny pane to zero (0x0 means hidden to the host)", () => {
        expect(insetForSizeBadge({ x: 0, y: 0, width: 300, height: 20 }, 2).height).toBe(1);
    });
});
