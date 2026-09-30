// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode, ToolNode } from "../types";
import { createPromotionClock } from "./promotion-clock";
import { TOOL_PROMOTION_MS } from "./tool-adapter";

const runningBash = (id: string, startedAt: number): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    status: "running",
    // Not a sleep: sleep-detect.ts promotes a whole-command sleep at once.
    params: { command: "cargo test -p agentmux-srv" },
    collapsed: false,
    summary: "",
    timestamp: startedAt,
});

/**
 * Build the clock inside a reactive root and return it. The clock's effect
 * first runs when the root's function returns, so time is driven from outside.
 */
function mount(initial: DocumentNode[]) {
    return createRoot((dispose) => {
        const [nodes, setNodes] = createSignal<DocumentNode[]>(initial);
        return { tick: createPromotionClock(nodes), setNodes, dispose };
    });
}

describe("createPromotionClock", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.setSystemTime(0);
    });
    afterEach(() => vi.useRealTimers());

    it("ticks once when a running call becomes due for promotion", () => {
        const c = mount([runningBash("a", 0)]);
        expect(c.tick()).toBe(0);
        vi.advanceTimersByTime(TOOL_PROMOTION_MS - 1);
        expect(c.tick()).toBe(0);
        vi.advanceTimersByTime(100);
        expect(c.tick()).toBe(1);
        c.dispose();
    });

    it("ticks again for the next-earliest pending promotion", () => {
        const c = mount([runningBash("a", 0), runningBash("b", 10_000)]);
        vi.advanceTimersByTime(TOOL_PROMOTION_MS + 100);
        expect(c.tick()).toBe(1);
        vi.advanceTimersByTime(10_000);
        expect(c.tick()).toBe(2);
        c.dispose();
    });

    it("doesn't tick with nothing running", () => {
        const c = mount([]);
        vi.advanceTimersByTime(TOOL_PROMOTION_MS * 3);
        expect(c.tick()).toBe(0);
        c.dispose();
    });

    it("reschedules when the document changes", () => {
        const c = mount([]);
        vi.advanceTimersByTime(5_000);
        c.setNodes([runningBash("a", 5_000)]);
        vi.advanceTimersByTime(TOOL_PROMOTION_MS + 100);
        expect(c.tick()).toBe(1);
        c.dispose();
    });

    it("stops its timer when its owner is disposed", () => {
        const c = mount([runningBash("a", 0)]);
        c.dispose();
        vi.advanceTimersByTime(TOOL_PROMOTION_MS * 2);
        expect(c.tick()).toBe(0);
        expect(vi.getTimerCount()).toBe(0);
    });
});
