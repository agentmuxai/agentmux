// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { type MoveHereDeps, moveOpenPaneHere } from "./move-singleton-here";

function deps(over: Partial<MoveHereDeps> = {}): MoveHereDeps & { calls: string[] } {
    const calls: string[] = [];
    return {
        calls,
        findOpen: async () => "open-block",
        loneNodeId: (blockId) => `node-of-${blockId}`,
        swap: (a, b) => calls.push(`swap ${a} ${b}`),
        show: (blockId) => calls.push(`show ${blockId}`),
        ...over,
    };
}

describe("moveOpenPaneHere", () => {
    it("swaps the open pane into this pane's place, closes this pane, then shows the moved one", async () => {
        const d = deps();
        const closeOwn = vi.fn(async () => {
            d.calls.push("close");
        });
        expect(await moveOpenPaneHere("own-block", "settings", closeOwn, d)).toBe(true);
        expect(d.calls).toEqual(["swap node-of-own-block node-of-open-block", "close", "show open-block"]);
    });

    it("does nothing when no pane of the view is open, or the open one is this pane", async () => {
        for (const found of [null, "own-block"]) {
            const d = deps({ findOpen: async () => found });
            const closeOwn = vi.fn(async () => {});
            expect(await moveOpenPaneHere("own-block", "settings", closeOwn, d)).toBe(false);
            expect(d.calls).toEqual([]);
            expect(closeOwn).not.toHaveBeenCalled();
        }
    });

    it("leaves the open pane where it is when it is in a stack or another tab (no lone node)", async () => {
        const d = deps({ loneNodeId: (blockId) => (blockId === "open-block" ? undefined : "node-of-own") });
        const closeOwn = vi.fn(async () => {});
        expect(await moveOpenPaneHere("own-block", "settings", closeOwn, d)).toBe(false);
        expect(d.calls).toEqual([]);
        expect(closeOwn).not.toHaveBeenCalled();
    });

    it("leaves everything when this pane is not a lone pane either", async () => {
        const d = deps({ loneNodeId: (blockId) => (blockId === "own-block" ? undefined : "node-of-open") });
        expect(await moveOpenPaneHere("own-block", "settings", async () => {}, d)).toBe(false);
        expect(d.calls).toEqual([]);
    });
});
