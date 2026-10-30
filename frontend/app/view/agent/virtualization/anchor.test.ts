// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    captureTopmostAnchor,
    isNearTop,
    NEAR_TOP_THRESHOLD_PX,
    restoreScrollFromAnchor,
    type ScrollAnchor,
} from "./anchor";

describe("captureTopmostAnchor", () => {
    it("returns null when no nodes are visible", () => {
        expect(captureTopmostAnchor([], 100)).toBeNull();
    });

    it("captures the first visible node id and offset relative to scrollTop", () => {
        // Topmost visible node is at offsetPx=200, scrollTop=250 →
        // anchor's offset = 250 - 200 = +50 (node is 50px above viewport top).
        const anchor = captureTopmostAnchor(
            [
                { id: "n5", offsetPx: 200 },
                { id: "n6", offsetPx: 280 },
            ],
            250,
        );
        expect(anchor).toEqual<ScrollAnchor>({ nodeId: "n5", offsetPx: 50 });
    });

    it("handles negative offset (anchor below viewport top — rare but valid)", () => {
        const anchor = captureTopmostAnchor([{ id: "n0", offsetPx: 500 }], 200);
        expect(anchor).toEqual<ScrollAnchor>({ nodeId: "n0", offsetPx: -300 });
    });
});

describe("restoreScrollFromAnchor", () => {
    it("restores the captured offset relative to the new node position", () => {
        // Anchor was captured at offset +50 from node n5.
        // After prepend, n5 is now at offsetPx=520 (was 200 — 320px of new content above).
        // Expected scrollTop: 520 + 50 = 570 (so n5 still appears 50px above viewport).
        const anchor: ScrollAnchor = { nodeId: "n5", offsetPx: 50 };
        expect(restoreScrollFromAnchor(anchor, 520)).toBe(570);
    });

    it("clamps negative results to 0", () => {
        // If anchor is below viewport (-300) and node moves up to offset 100,
        // raw result would be -200 → clamp to 0.
        const anchor: ScrollAnchor = { nodeId: "n0", offsetPx: -300 };
        expect(restoreScrollFromAnchor(anchor, 100)).toBe(0);
    });

    it("round-trips: captureTopmostAnchor → restoreScrollFromAnchor (no-prepend identity)", () => {
        const visible = [{ id: "n5", offsetPx: 200 }];
        const anchor = captureTopmostAnchor(visible, 250)!;
        // Same node still at offsetPx=200 (no prepend). Restore should give back 250.
        expect(restoreScrollFromAnchor(anchor, 200)).toBe(250);
    });
});

describe("isNearTop", () => {
    it("returns true when scrollTop < threshold", () => {
        expect(isNearTop(0)).toBe(true);
        expect(isNearTop(49)).toBe(true);
    });

    it("returns false when scrollTop >= threshold", () => {
        expect(isNearTop(50)).toBe(false);
        expect(isNearTop(1000)).toBe(false);
    });

    it("uses NEAR_TOP_THRESHOLD_PX as default", () => {
        expect(NEAR_TOP_THRESHOLD_PX).toBe(50);
    });
});
