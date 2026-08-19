// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { getCacheHitRate, getTotal, recordTurn, resetSession } from "./token-usage";

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
