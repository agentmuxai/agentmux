// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Which Editor viewers take the pane's zoom on their content, and the viewer's hand-off of its
 * scroller and anchor to the Editor's zoom anchor.
 */

import { describe, expect, it } from "vitest";
import { contentZoomOf, hasContentZoom, registerContentZoom, type ContentZoomHandle } from "./content-zoom";

const handle = (): ContentZoomHandle => ({ scroller: () => undefined, adapter: { capture: () => null, restore: () => true } });

describe("content zoom", () => {
    it("is the PDF viewer's; the text editor and the other viewers zoom as a whole pane", () => {
        expect(hasContentZoom("pdf")).toBe(true);
        expect(hasContentZoom("text")).toBe(false);
        expect(hasContentZoom("image")).toBe(false);
    });

    it("hands the viewer in front to the zoom anchor, and lets go of it", () => {
        const h = handle();
        const release = registerContentZoom("b1", h);
        expect(contentZoomOf("b1")).toBe(h);
        expect(contentZoomOf("b2")).toBeUndefined();
        release();
        expect(contentZoomOf("b1")).toBeUndefined();
    });

    it("keeps a newer viewer when an older one lets go late", () => {
        const older = handle();
        const newer = handle();
        const releaseOlder = registerContentZoom("b1", older);
        registerContentZoom("b1", newer);
        releaseOlder();
        expect(contentZoomOf("b1")).toBe(newer);
    });
});
