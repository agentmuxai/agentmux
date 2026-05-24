// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * L1 unit tests for the hover-anchor direction picker.
 *
 * Spec: `docs/specs/SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md` §6.1.
 */

import { describe, expect, it } from "vitest";
import { pickExpandDirection } from "./hover-anchor";

describe("pickExpandDirection", () => {
    const viewportH = 1000;

    it("returns 'below' when there's plenty of space below", () => {
        // Summary near top; 100px below summary, 1000-100=900px below.
        const rect = { top: 50, bottom: 100 };
        expect(pickExpandDirection(rect, viewportH, 400)).toBe("below");
    });

    it("returns 'above' when body doesn't fit below but fits above", () => {
        // Summary near bottom; 1000-950=50px below, 900px above.
        const rect = { top: 900, bottom: 950 };
        expect(pickExpandDirection(rect, viewportH, 400)).toBe("above");
    });

    it("returns 'below' on tie (equal space, body fits in both)", () => {
        // Summary in the middle; 475px below, 475px above. Body needs 200.
        const rect = { top: 475, bottom: 525 };
        expect(pickExpandDirection(rect, viewportH, 200)).toBe("below");
    });

    it("returns 'below' when body fits below — even if above has more space", () => {
        // Summary lower-middle; below = 1000-700=300, above = 650.
        // Body = 200, fits below → step 1 picks below despite above
        // having more room.
        const rect = { top: 650, bottom: 700 };
        expect(pickExpandDirection(rect, viewportH, 200)).toBe("below");
    });

    it("picks the side with more room when body fits in neither", () => {
        // Body = 600. Summary near bottom: below = 100, above = 850.
        // Neither side fits 600, but above has more room.
        const rect = { top: 850, bottom: 900 };
        expect(pickExpandDirection(rect, viewportH, 600)).toBe("above");
    });

    it("picks the larger side when body fits neither — below wins on tie", () => {
        // Exact-middle summary, body too tall for either half.
        const rect = { top: 475, bottom: 525 };
        expect(pickExpandDirection(rect, viewportH, 999)).toBe("below");
    });

    it("handles a summary scrolled partly off the top of the viewport", () => {
        // Summary's `top` is negative (off-screen). spaceAbove clamps
        // to 0; body must go below.
        const rect = { top: -20, bottom: 30 };
        expect(pickExpandDirection(rect, viewportH, 400)).toBe("below");
    });

    it("handles a summary scrolled past the bottom of the viewport", () => {
        // Summary's `bottom` exceeds viewportH. spaceBelow clamps to
        // 0; body must go above (assuming there's room).
        const rect = { top: 990, bottom: 1020 };
        expect(pickExpandDirection(rect, viewportH, 400)).toBe("above");
    });

    it("handles a zero-height viewport gracefully (degenerate but defined)", () => {
        // Both spaces are 0; tie → "below".
        const rect = { top: 0, bottom: 0 };
        expect(pickExpandDirection(rect, 0, 100)).toBe("below");
    });

    it("handles a zero-height body — always picks below (step 1)", () => {
        // 0 fits in any spaceBelow >= 0.
        expect(pickExpandDirection({ top: 100, bottom: 200 }, viewportH, 0)).toBe("below");
        // Even when summary is at the very bottom and spaceBelow is 0,
        // step 1 still matches (0 <= 0), so "below".
        expect(pickExpandDirection({ top: 990, bottom: 1000 }, viewportH, 0)).toBe("below");
    });
});
