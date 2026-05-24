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
    // Container occupying the whole viewport (no clipping).
    const fullViewport = { top: 0, bottom: 1000 };

    it("returns 'below' when there's plenty of space below", () => {
        // Summary near top; 1000-100=900px below.
        const rect = { top: 50, bottom: 100 };
        expect(pickExpandDirection(rect, fullViewport, 400)).toBe("below");
    });

    it("returns 'above' when body doesn't fit below but fits above", () => {
        // Summary near bottom; 1000-950=50px below, 900px above.
        const rect = { top: 900, bottom: 950 };
        expect(pickExpandDirection(rect, fullViewport, 400)).toBe("above");
    });

    it("returns 'below' on tie (equal space, body fits in both)", () => {
        const rect = { top: 475, bottom: 525 };
        expect(pickExpandDirection(rect, fullViewport, 200)).toBe("below");
    });

    it("returns 'below' when body fits below — even if above has more space", () => {
        const rect = { top: 650, bottom: 700 };
        expect(pickExpandDirection(rect, fullViewport, 200)).toBe("below");
    });

    it("picks the side with more room when body fits in neither", () => {
        const rect = { top: 850, bottom: 900 };
        expect(pickExpandDirection(rect, fullViewport, 600)).toBe("above");
    });

    it("picks the larger side when body fits neither — below wins on tie", () => {
        const rect = { top: 475, bottom: 525 };
        expect(pickExpandDirection(rect, fullViewport, 999)).toBe("below");
    });

    it("handles a summary scrolled partly off the top", () => {
        const rect = { top: -20, bottom: 30 };
        expect(pickExpandDirection(rect, fullViewport, 400)).toBe("below");
    });

    it("handles a summary scrolled past the bottom", () => {
        const rect = { top: 990, bottom: 1020 };
        expect(pickExpandDirection(rect, fullViewport, 400)).toBe("above");
    });

    it("handles a zero-height container gracefully", () => {
        const rect = { top: 0, bottom: 0 };
        expect(pickExpandDirection(rect, { top: 0, bottom: 0 }, 100)).toBe("below");
    });

    it("handles a zero-height body — always picks below (step 1)", () => {
        expect(
            pickExpandDirection({ top: 100, bottom: 200 }, fullViewport, 0),
        ).toBe("below");
        expect(
            pickExpandDirection({ top: 990, bottom: 1000 }, fullViewport, 0),
        ).toBe("below");
    });

    // Codex P1 round 2 on PR #1021 — the agent pane is a
    // scrollable region, not the whole viewport. The container
    // rect lets us pick the right direction when the pane's
    // bottom is well above the window's bottom.
    describe("clipped to a scroll container", () => {
        // Pane occupies the upper 600px of a 1000px window — e.g.
        // a split view where the lower 400px is another pane.
        const upperPane = { top: 0, bottom: 600 };

        it("picks 'above' when the summary is near the pane's bottom even though the window has plenty of room below", () => {
            // Summary in viewport coords at y=550-580. Window has
            // 420px below (1000-580), but pane only has 20px below
            // (600-580). Above the summary inside the pane: 550px.
            const rect = { top: 550, bottom: 580 };
            expect(pickExpandDirection(rect, upperPane, 400)).toBe("above");
        });

        it("picks 'below' when the summary is near the pane's top", () => {
            const rect = { top: 30, bottom: 60 };
            expect(pickExpandDirection(rect, upperPane, 400)).toBe("below");
        });

        it("handles a pane that doesn't start at 0", () => {
            // Pane occupies y=200 to y=700 (a header offsets it).
            const middlePane = { top: 200, bottom: 700 };
            // Summary at y=600-630: 70px below, 400px above inside
            // the pane. 400-tall body fits above.
            const rect = { top: 600, bottom: 630 };
            expect(pickExpandDirection(rect, middlePane, 400)).toBe("above");
        });

        it("clamps to 0 when the summary is outside the container", () => {
            // Summary above the pane's top: spaceAbove negative,
            // clamped to 0. Below has the full pane height.
            const rect = { top: 150, bottom: 180 };
            const middlePane = { top: 200, bottom: 700 };
            expect(pickExpandDirection(rect, middlePane, 300)).toBe("below");
        });
    });
});
