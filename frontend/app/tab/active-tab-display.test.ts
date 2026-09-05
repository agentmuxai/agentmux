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
import { driveTabSelection, resolveDisplayActiveTabId } from "./active-tab-display";

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

describe("driveTabSelection (Codex P2 on PR #2993)", () => {
    /**
     * Fake of the real pair: a committed id that only moves when setActive
     * actually issues, and setActiveTab's OWN early-return guard, which is
     * what makes the naive one-call-per-click version lose the second click.
     */
    function harness(initialCommitted: string) {
        let committed = initialCommitted;
        let intent: string | null = null;
        const issued: string[] = [];
        let resolveGate: (() => void) | null = null;

        const setActive = async (tabId: string): Promise<void> => {
            // Mirrors store/tab-actions.ts: `if (fromTabId === tabId) return;`
            if (committed === tabId) return;
            issued.push(tabId);
            if (resolveGate) {
                await new Promise<void>((r) => {
                    const prev = resolveGate!;
                    resolveGate = () => {
                        prev();
                        r();
                    };
                });
            }
            committed = tabId;
        };

        return {
            issued,
            committed: () => committed,
            latestIntent: () => intent,
            click: (id: string) => (intent = id),
            deps: () => ({ latestIntent: () => intent, committed: () => committed, setActive }),
        };
    }

    it("a plain switch issues exactly one RPC and lands on the target", async () => {
        const h = harness("a");
        h.click("b");
        await driveTabSelection(h.deps());
        expect(h.issued).toEqual(["b"]);
        expect(h.committed()).toBe("b");
    });

    it("honours a click BACK to the committed tab made mid-flight", async () => {
        // THE Codex case. Committed is "a"; user clicks "b", then clicks "a"
        // again before b's RPC resolves. The naive version fires setActive(b),
        // and the second click is lost twice over: handleSelect's guard saw
        // committed === "a", and setActiveTab's own guard would have no-op'd
        // an "a" re-issue anyway. Content ended on "b" against the user's
        // last click.
        const h = harness("a");
        h.click("b");
        const running = driveTabSelection(h.deps());
        h.click("a"); // lands while setActive("b") is still in flight
        await running;
        expect(h.committed()).toBe("a");
        expect(h.issued).toEqual(["b", "a"]);
    });

    it("converges on the LAST of several mid-flight clicks", async () => {
        const h = harness("a");
        h.click("b");
        const running = driveTabSelection(h.deps());
        h.click("c");
        await running;
        expect(h.committed()).toBe("c");
    });

    it("does nothing when the intent already matches the committed tab", async () => {
        const h = harness("a");
        h.click("a");
        await driveTabSelection(h.deps());
        expect(h.issued).toEqual([]);
    });

    it("does nothing when there is no pending intent", async () => {
        const h = harness("a");
        await driveTabSelection(h.deps());
        expect(h.issued).toEqual([]);
    });

    it("propagates a failing switch rather than looping on it", async () => {
        let intent: string | null = "b";
        const deps = {
            latestIntent: () => intent,
            committed: () => "a",
            setActive: () => Promise.reject(new Error("rpc failed")),
        };
        await expect(driveTabSelection(deps)).rejects.toThrow("rpc failed");
    });
});
