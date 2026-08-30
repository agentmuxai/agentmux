// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { getAgentBreakdown, getAgentCacheHitRate, getCacheHitRate, getTotal, recordTurn, resetSession } from "./token-usage";

describe("token-usage store — cache-hit-rate normalization", () => {
    beforeEach(() => {
        resetSession();
    });

    it("computes the hit rate correctly for our own shape (input = freshInput + cacheCreation + cacheRead)", () => {
        // claude-translator.ts / useAgentStream.ts shape: `input` is the
        // collapsed total, `freshInput` carries the fresh-only portion.
        recordTurn("claude", { input: 1000, output: 50, freshInput: 100, cacheCreation: 300, cacheRead: 600 });
        expect(getCacheHitRate()).toBeCloseTo(0.6, 5); // 600 / (100 + 300 + 600)
    });

    it("reagentx/codex P2 on PR #2658 — normalizes the backend TokenCounts shape (input = fresh only, no freshInput field)", () => {
        // gotypes.d.ts's TokenCounts shape, as passed by
        // useNextPromptSuggestion/useAgentActivitySummary/ActivityDock/
        // swarm-view's ambient `result.tokens` — `input` means fresh-only
        // here, and there is no separate `freshInput` field at all.
        recordTurn("ambient:next_prompt_suggestion", { input: 100, output: 5, cacheCreation: 300, cacheRead: 600 });
        // Before the fix this read 1.0 (600 / (300 + 600), silently
        // dropping the 100 fresh tokens from the denominator because they
        // never appear under a `freshInput` key). Correct answer treats
        // `input` as the fresh contribution: 600 / (100 + 300 + 600).
        expect(getCacheHitRate()).toBeCloseTo(0.6, 5);
    });

    it("mixes both shapes across services without corrupting either one's contribution", () => {
        recordTurn("claude", { input: 1000, output: 0, freshInput: 100, cacheCreation: 300, cacheRead: 600 });
        recordTurn("ambient:next_prompt_suggestion", { input: 50, output: 0, cacheCreation: 0, cacheRead: 450 });
        // fresh: 100 + 50 = 150; cacheCreation: 300 + 0 = 300; cacheRead: 600 + 450 = 1050
        // total prompt: 150 + 300 + 1050 = 1500; hit rate: 1050 / 1500 = 0.7
        expect(getCacheHitRate()).toBeCloseTo(0.7, 5);
    });

    it("returns null when no service has reported a cache breakdown yet", () => {
        recordTurn("codex", { input: 500, output: 20 });
        expect(getCacheHitRate()).toBeNull();
    });

    it("getTotal still sums the plain input/output regardless of breakdown shape", () => {
        recordTurn("claude", { input: 1000, output: 50, freshInput: 100, cacheCreation: 300, cacheRead: 600 });
        recordTurn("ambient:next_prompt_suggestion", { input: 50, output: 5, cacheCreation: 0, cacheRead: 450 });
        const total = getTotal();
        expect(total.input).toBe(1050);
        expect(total.output).toBe(55);
    });
});

describe("token-usage store — by-agent breakdown (SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md)", () => {
    beforeEach(() => {
        resetSession();
    });

    it("keys a real agent turn by blockId and carries its agentName/costUsd/turn count", () => {
        recordTurn("claude", { input: 1000, output: 50 }, { blockId: "block-1", agentName: "Manoz", costUsd: 0.12 });
        recordTurn("claude", { input: 500, output: 20 }, { blockId: "block-1", agentName: "Manoz", costUsd: 0.05 });
        const rows = getAgentBreakdown();
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({
            blockId: "block-1",
            agentName: "Manoz",
            isAmbient: false,
            input: 1500,
            output: 70,
            numTurns: 2,
        });
        expect(rows[0].costUsd).toBeCloseTo(0.17, 5);
    });

    it("collapses turns with no agent context into a single ambient bucket", () => {
        recordTurn("ambient:next_prompt_suggestion", { input: 100, output: 5 });
        recordTurn("ambient:activity_summary", { input: 40, output: 2 });
        const rows = getAgentBreakdown();
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({
            blockId: null,
            isAmbient: true,
            agentName: "AgentMux internal",
            input: 140,
            output: 7,
            numTurns: 2,
        });
    });

    it("sorts real agents before the ambient bucket regardless of size", () => {
        recordTurn("ambient:next_prompt_suggestion", { input: 100_000, output: 5_000 });
        recordTurn("claude", { input: 10, output: 1 }, { blockId: "block-1", agentName: "Manoz", costUsd: 0.01 });
        const rows = getAgentBreakdown();
        expect(rows.map((r) => r.isAmbient)).toEqual([false, true]);
    });

    it("sorts real agents by cost descending when any row has a nonzero cost", () => {
        recordTurn("claude", { input: 100, output: 10 }, { blockId: "block-cheap", agentName: "Cheap", costUsd: 0.01 });
        recordTurn("claude", { input: 100, output: 10 }, { blockId: "block-pricey", agentName: "Pricey", costUsd: 0.5 });
        const rows = getAgentBreakdown();
        expect(rows.map((r) => r.agentName)).toEqual(["Pricey", "Cheap"]);
    });

    it("falls back to token total ordering when no agent has reported a cost", () => {
        recordTurn("codex", { input: 100, output: 10 }, { blockId: "block-small", agentName: "Small" });
        recordTurn("codex", { input: 900, output: 90 }, { blockId: "block-big", agentName: "Big" });
        const rows = getAgentBreakdown();
        expect(rows.map((r) => r.agentName)).toEqual(["Big", "Small"]);
    });

    it("re-asserts agentName on later turns so an early unresolved name doesn't strand the fallback", () => {
        recordTurn("claude", { input: 10, output: 1 }, { blockId: "block-1", agentName: "block-1" });
        recordTurn("claude", { input: 10, output: 1 }, { blockId: "block-1", agentName: "Manoz" });
        const rows = getAgentBreakdown();
        expect(rows[0].agentName).toBe("Manoz");
    });

    it("computes a per-agent cache hit rate independent of other agents' breakdowns", () => {
        recordTurn("claude", { input: 1000, output: 50, freshInput: 100, cacheCreation: 0, cacheRead: 900 }, { blockId: "block-1", agentName: "Manoz" });
        recordTurn("codex", { input: 500, output: 20 }, { blockId: "block-2", agentName: "Other" });
        const rows = getAgentBreakdown();
        const manoz = rows.find((r) => r.blockId === "block-1")!;
        const other = rows.find((r) => r.blockId === "block-2")!;
        expect(getAgentCacheHitRate(manoz)).toBeCloseTo(0.9, 5);
        expect(getAgentCacheHitRate(other)).toBeNull();
    });

    it("resetSession clears byAgent alongside byService", () => {
        recordTurn("claude", { input: 10, output: 1 }, { blockId: "block-1", agentName: "Manoz" });
        resetSession();
        expect(getAgentBreakdown()).toHaveLength(0);
    });
});
