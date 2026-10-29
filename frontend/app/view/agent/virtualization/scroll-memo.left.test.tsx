// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane that goes away scrolled up (unmounted behind a tab, or closed) leaves its position for the
 * next pane of the same agent (scroll-memo.ts, `left`). Without a store the memo is kept by block id.
 */
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hasScrolledUpMemo, resetLeftScrollMemosForTests, useScrollMemo } from "./scroll-memo";

const agentOf = new Map<string, string>();
vi.mock("@/app/store/block-atom-cache", () => ({
    getBlockMetaKeyAtom: (blockId: string) => () => agentOf.get(blockId),
}));

/** A scroller whose rows sit at the given tops; the scroller's own top is 0. */
function scrollerWith(rows: { id: string; top: number }[]): HTMLElement {
    const el = document.createElement("div");
    for (const r of rows) {
        const row = document.createElement("div");
        row.dataset.nodeId = r.id;
        vi.spyOn(row, "getBoundingClientRect").mockReturnValue({ top: r.top, bottom: r.top + 50, left: 0, right: 100, width: 100, height: 50, x: 0, y: r.top, toJSON: () => ({}) });
        el.appendChild(row);
    }
    vi.spyOn(el, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 400, left: 0, right: 100, width: 100, height: 400, x: 0, y: 0, toJSON: () => ({}) });
    return el;
}

/** Mounts a pane's scroll memo; the result's `unmount` is the pane going away. */
function pane(blockId: string, scroller: HTMLElement, following: boolean) {
    const scrollToRow = vi.fn();
    const [ready, setReady] = createSignal(false);
    let dispose = () => {};
    createRoot((d) => {
        dispose = d;
        useScrollMemo({ blockId, scroller: () => scroller, following: () => following, historyReady: ready, scrollToRow });
    });
    return { scrollToRow, ready: () => setReady(true), unmount: () => dispose() };
}

beforeEach(() => {
    agentOf.clear();
    sessionStorage.clear();
    resetLeftScrollMemosForTests();
    vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
        cb();
        return 1;
    });
});
afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe("scroll-memo: a pane that goes away scrolled up leaves its place", () => {
    it("the next pane of the same agent, a new block, goes back to the row it was on", () => {
        agentOf.set("blk-1", "agent-a");
        agentOf.set("blk-2", "agent-a");
        const first = pane("blk-1", scrollerWith([{ id: "r1", top: -80 }, { id: "r2", top: 20 }]), false);
        first.unmount();
        expect(hasScrolledUpMemo("blk-2"), "the reopened pane (and its history load) is told").toBe(true);
        const second = pane("blk-2", scrollerWith([{ id: "r1", top: 0 }]), true);
        second.ready();
        expect(second.scrollToRow).toHaveBeenCalledWith("r2", -20);
        expect(hasScrolledUpMemo("blk-2"), "used once").toBe(false);
    });

    it("files the place under the agent even when the block's meta is already gone at cleanup", () => {
        agentOf.set("blk-1", "agent-a");
        agentOf.set("blk-2", "agent-a");
        const first = pane("blk-1", scrollerWith([{ id: "r2", top: 20 }]), false);
        agentOf.delete("blk-1"); // a deleted block: its meta is gone before the pane is disposed
        first.unmount();
        expect(hasScrolledUpMemo("blk-2")).toBe(true);
    });

    it("a pane that was following leaves nothing, and clears an older place", () => {
        agentOf.set("blk-1", "agent-a");
        pane("blk-1", scrollerWith([{ id: "r2", top: 20 }]), false).unmount();
        pane("blk-1", scrollerWith([{ id: "r2", top: 20 }]), true).unmount();
        expect(hasScrolledUpMemo("blk-1")).toBe(false);
    });

    it("another agent's pane is not moved", () => {
        agentOf.set("blk-1", "agent-a");
        agentOf.set("blk-9", "agent-b");
        pane("blk-1", scrollerWith([{ id: "r2", top: 20 }]), false).unmount();
        const other = pane("blk-9", scrollerWith([{ id: "r2", top: 20 }]), true);
        other.ready();
        expect(other.scrollToRow).not.toHaveBeenCalled();
    });

    it("keeps the block's own place when the pane has no agent id", () => {
        pane("blk-7", scrollerWith([{ id: "r3", top: 10 }]), false).unmount();
        expect(hasScrolledUpMemo("blk-7")).toBe(true);
        expect(hasScrolledUpMemo("blk-8")).toBe(false);
    });

    it("remembers the most recent agents only", () => {
        for (let i = 0; i < 105; i++) {
            agentOf.set(`b${i}`, `a${i}`);
            pane(`b${i}`, scrollerWith([{ id: "r", top: 10 }]), false).unmount();
        }
        expect(hasScrolledUpMemo("b0"), "the oldest was dropped").toBe(false);
        expect(hasScrolledUpMemo("b104")).toBe(true);
    });
});
