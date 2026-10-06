// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, test } from "vitest";
import {
    compactionThreshold,
    contextWindowForModel,
    nextTierAbove,
} from "./context-window";

describe("contextWindowForModel", () => {
    test("Opus / Fable resolve to 1M", () => {
        expect(contextWindowForModel("claude-opus-4-8")).toBe(1_000_000);
        expect(contextWindowForModel("opus")).toBe(1_000_000);
        expect(contextWindowForModel("claude-fable-5")).toBe(1_000_000);
    });
    test("Haiku resolves to 200K", () => {
        expect(contextWindowForModel("claude-haiku-4-5")).toBe(200_000);
    });
    test("Sonnet 4.x seeds conservatively at 200K (learns up to 1M)", () => {
        expect(contextWindowForModel("claude-sonnet-4-6")).toBe(200_000);
        expect(contextWindowForModel("claude-sonnet-4-5")).toBe(200_000);
    });
    test("Sonnet 5+ has no beta gate — seeds at 1M directly", () => {
        expect(contextWindowForModel("claude-sonnet-5")).toBe(1_000_000);
        expect(contextWindowForModel("CLAUDE-SONNET-5")).toBe(1_000_000);
    });
    test("bare 'sonnet' family alias (unresolved) stays conservative", () => {
        expect(contextWindowForModel("sonnet")).toBe(200_000);
    });
    test("a [1m] model is 1M whatever the family", () => {
        expect(contextWindowForModel("claude-sonnet-4-6[1m]")).toBe(1_000_000);
        expect(contextWindowForModel("sonnet[1m]")).toBe(1_000_000);
    });
    test("unknown / non-Claude models are undefined (caller falls back)", () => {
        expect(contextWindowForModel("gpt-5-codex")).toBeUndefined();
        expect(contextWindowForModel("gemini-3-pro")).toBeUndefined();
        expect(contextWindowForModel(undefined)).toBeUndefined();
        expect(contextWindowForModel(null)).toBeUndefined();
    });
});

describe("nextTierAbove", () => {
    test("is the smallest known tier strictly above the prompt", () => {
        expect(nextTierAbove(50_000)).toBe(200_000);
        expect(nextTierAbove(200_000)).toBe(1_000_000);
        expect(nextTierAbove(250_000)).toBe(1_000_000);
    });
    test("is undefined above every tier, never the prompt itself", () => {
        // The pre-fix version returned the observed value, so one bad reading
        // (a turn total of 17M) set the window to 17M for good.
        expect(nextTierAbove(1_000_000)).toBeUndefined();
        expect(nextTierAbove(17_000_000)).toBeUndefined();
    });
});

describe("compactionThreshold", () => {
    test("sits ~33K below the window", () => {
        expect(compactionThreshold(1_000_000)).toBe(967_000);
        expect(compactionThreshold(200_000)).toBe(167_000);
    });
    test("never negative for tiny windows", () => {
        expect(compactionThreshold(10_000)).toBe(1);
    });
});
