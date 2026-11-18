// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { narrowTiersFor, PLACES_HIDDEN_TO, PREVIEW_HIDDEN_TO } from "./files-narrow";

describe("narrowTiersFor", () => {
    it("drops Places at 480 px and the preview at 640 px, inclusive, at 100%", () => {
        expect(narrowTiersFor(PLACES_HIDDEN_TO + 1, 1)).toEqual({ places: false, preview: true });
        expect(narrowTiersFor(PLACES_HIDDEN_TO, 1)).toEqual({ places: true, preview: true });
        expect(narrowTiersFor(PREVIEW_HIDDEN_TO + 1, 1)).toEqual({ places: false, preview: false });
        expect(narrowTiersFor(PREVIEW_HIDDEN_TO, 1)).toEqual({ places: false, preview: true });
    });

    it("counts the widths in the pane's own zoomed pixels, as when the whole pane was zoomed", () => {
        // 900 px wide: roomy at 100%, but at 200% it is a 450-px pane to the list, so no Places, and no preview.
        expect(narrowTiersFor(900, 1)).toEqual({ places: false, preview: false });
        expect(narrowTiersFor(900, 2)).toEqual({ places: true, preview: true });
        // Zoomed out, the same width has room for both.
        expect(narrowTiersFor(400, 0.5)).toEqual({ places: false, preview: false });
    });

    it("hides nothing for a width not measured yet", () => {
        expect(narrowTiersFor(0, 1)).toEqual({ places: false, preview: false });
        expect(narrowTiersFor(Number.NaN, 1)).toEqual({ places: false, preview: false });
    });

    it("falls back to the width itself for a level that is not a positive number", () => {
        expect(narrowTiersFor(500, 0)).toEqual({ places: false, preview: true });
    });
});
