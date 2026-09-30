// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveDisplayedTabId, tabContainerVisibility } from "./window-tab-visibility";

describe("tabContainerVisibility", () => {
    it("by default skips an inactive tab's layout with content-visibility", () => {
        expect(tabContainerVisibility(false, false, false)).toEqual({
            "content-visibility": "hidden",
            visibility: null,
            opacity: null,
            "pointer-events": "none",
            hiddenLaidOut: false,
        });
    });

    it("kept laid out, hides an inactive tab with visibility and keeps it laid out", () => {
        expect(tabContainerVisibility(false, true, false)).toEqual({
            "content-visibility": "visible",
            visibility: "hidden",
            // Nothing inside can paint through (a `visibility` transition would).
            opacity: "0",
            "pointer-events": "none",
            hiddenLaidOut: true,
        });
    });

    it("shows the displayed tab the same way in both modes", () => {
        for (const keep of [false, true]) {
            expect(tabContainerVisibility(true, keep, false)).toEqual({
                "content-visibility": "visible",
                visibility: null,
                opacity: null,
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

// ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §5.2: a warm destination
// shows from the switch intent, before the round trip; anything else waits.
describe("resolveDisplayedTabId", () => {
    const base = {
        committed: "a",
        intent: "b" as string | null,
        tabIds: ["a", "b", "c"],
        keepLaidOut: true,
        wasShown: (id: string) => id !== "c",
    };

    it("shows a warm destination before the backend commits it", () => {
        expect(resolveDisplayedTabId(base)).toBe("b");
    });

    it("waits for the backend with no switch in flight, or once it has caught up", () => {
        expect(resolveDisplayedTabId({ ...base, intent: null })).toBe("a");
        expect(resolveDisplayedTabId({ ...base, committed: "b" })).toBe("b");
    });

    it("waits for a tab not shown yet: its first reveal stays gated", () => {
        expect(resolveDisplayedTabId({ ...base, intent: "c" })).toBe("a");
    });

    it("waits with the setting off: hidden tabs are not kept laid out", () => {
        expect(resolveDisplayedTabId({ ...base, keepLaidOut: false })).toBe("a");
    });

    it("ignores an intent for a tab this window no longer has", () => {
        expect(resolveDisplayedTabId({ ...base, intent: "gone", wasShown: () => true })).toBe("a");
    });
});
