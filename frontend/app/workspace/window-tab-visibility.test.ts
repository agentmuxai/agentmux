// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { tabContainerVisibility } from "./window-tab-visibility";

describe("tabContainerVisibility", () => {
    it("by default skips an inactive tab's layout with content-visibility", () => {
        expect(tabContainerVisibility(false, false, false)).toEqual({
            "content-visibility": "hidden",
            visibility: null,
            "pointer-events": "none",
            hiddenLaidOut: false,
        });
    });

    it("kept laid out, hides an inactive tab with visibility and keeps it laid out", () => {
        expect(tabContainerVisibility(false, true, false)).toEqual({
            "content-visibility": "visible",
            visibility: "hidden",
            "pointer-events": "none",
            hiddenLaidOut: true,
        });
    });

    it("shows the displayed tab the same way in both modes", () => {
        for (const keep of [false, true]) {
            expect(tabContainerVisibility(true, keep, false)).toEqual({
                "content-visibility": "visible",
                visibility: null,
                "pointer-events": "auto",
                hiddenLaidOut: false,
            });
        }
    });

    it("lets the reveal gate hide the displayed tab in both modes", () => {
        for (const keep of [false, true]) {
            const v = tabContainerVisibility(true, keep, true);
            expect(v.visibility).toBe("hidden");
            expect(v.hiddenLaidOut).toBe(false);
        }
    });
});
