// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the tab strip's highlight resolution —
 * docs/specs/SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04.md.
 *
 * The property that matters: the pill must be able to show the clicked tab
 * WITHOUT the workspace's committed `activetabid` having moved yet, because
 * that value only lands after the SetActiveTab RPC round trip — and its
 * arrival is what triggers the destination's `display:none → flex` reveal,
 * whose browser-side layout cost scales with the destination's size.
 */

import { describe, expect, it } from "vitest";
import { resolveDisplayActiveTabId } from "./active-tab-display";

const resolve = (i: Partial<Parameters<typeof resolveDisplayActiveTabId>[0]>) =>
    resolveDisplayActiveTabId({
        realActiveTabId: "a",
        allTabIds: ["a", "b", "c"],
        hiddenTabIds: new Set<string>(),
        pendingSelectedTabId: null,
        ...i,
    });

describe("resolveDisplayActiveTabId", () => {
    describe("optimistic select (the fix)", () => {
        it("highlights the clicked tab while the backend still says otherwise", () => {
            // THE flagship case. realActiveTabId is still "a" because the
            // SetActiveTab RPC has not round-tripped; the pill must already
            // read "b" regardless.
            expect(resolve({ realActiveTabId: "a", pendingSelectedTabId: "b" })).toBe("b");
        });

        it("falls back to the committed id once the pending select clears", () => {
            expect(resolve({ realActiveTabId: "b", pendingSelectedTabId: null })).toBe("b");
        });

        it("ignores a pending select for a tab that no longer exists", () => {
            // The workspace updated and dropped the tab out from under an
            // in-flight select — highlighting it would point at a pill the
            // strip does not render.
            expect(resolve({ realActiveTabId: "a", allTabIds: ["a", "c"], pendingSelectedTabId: "b" })).toBe("a");
        });

        it("ignores a pending select for a tab that is now being closed", () => {
            expect(
                resolve({
                    realActiveTabId: "a",
                    hiddenTabIds: new Set(["b"]),
                    pendingSelectedTabId: "b",
                }),
            ).toBe("a");
        });
    });

    describe("close-flow promotion (must keep working — SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH §9)", () => {
        it("promotes the right-hand neighbor when the active tab is mid-close", () => {
            expect(resolve({ realActiveTabId: "b", hiddenTabIds: new Set(["b"]) })).toBe("c");
        });

        it("falls back to the left neighbor when the closed tab was rightmost", () => {
            expect(resolve({ realActiveTabId: "c", hiddenTabIds: new Set(["c"]) })).toBe("b");
        });

        it("skips over several hidden tabs to find a live one", () => {
            expect(
                resolve({
                    realActiveTabId: "a",
                    allTabIds: ["a", "b", "c"],
                    hiddenTabIds: new Set(["a", "b"]),
                }),
            ).toBe("c");
        });

        it("returns the real id when every tab is hidden", () => {
            expect(
                resolve({ realActiveTabId: "a", hiddenTabIds: new Set(["a", "b", "c"]) }),
            ).toBe("a");
        });

        it("leaves the active tab alone when some OTHER tab is closing", () => {
            expect(resolve({ realActiveTabId: "a", hiddenTabIds: new Set(["c"]) })).toBe("a");
        });
    });

    describe("precedence when a select and a close overlap", () => {
        it("the user's own click outranks the inferred neighbor promotion", () => {
            // Active tab "b" is mid-close (promotion would say "c"), but the
            // user has since clicked "a". Their explicit choice wins.
            expect(
                resolve({
                    realActiveTabId: "b",
                    hiddenTabIds: new Set(["b"]),
                    pendingSelectedTabId: "a",
                }),
            ).toBe("a");
        });
    });
});
