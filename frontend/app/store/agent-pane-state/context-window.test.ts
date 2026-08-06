// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, test } from "vitest";
import {
    compactionThreshold,
    contextWindowForModel,
    learnContextWindow,
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
    test("unknown / non-Claude models are undefined (caller falls back)", () => {
        expect(contextWindowForModel("gpt-5-codex")).toBeUndefined();
        expect(contextWindowForModel("gemini-3-pro")).toBeUndefined();
        expect(contextWindowForModel(undefined)).toBeUndefined();
        expect(contextWindowForModel(null)).toBeUndefined();
    });
});

describe("learnContextWindow", () => {
    test("seeds from the model on first observation", () => {
        expect(learnContextWindow(null, 1_000, "claude-opus-4-8")).toBe(1_000_000);
        expect(learnContextWindow(null, 1_000, "claude-haiku-4-5")).toBe(200_000);
    });
    test("Sonnet-4.x-1M-beta: promotes 200K → 1M once context exceeds 200K", () => {
        const seed = learnContextWindow(null, 50_000, "claude-sonnet-4-6");
        expect(seed).toBe(200_000);
        // a later turn whose prompt exceeds the 200K seed proves it's the 1M variant
        expect(learnContextWindow(seed, 250_000, "claude-sonnet-4-6")).toBe(1_000_000);
    });
    test("Sonnet 5 seeds at 1M on first observation — no learning needed", () => {
        expect(learnContextWindow(null, 50_000, "claude-sonnet-5")).toBe(1_000_000);
    });
    test("model switch mid-session Sonnet-4.6(learned 1M) → Sonnet 5 stays at 1M", () => {
        expect(learnContextWindow(1_000_000, 50_000, "claude-sonnet-5", "claude-sonnet-4-6")).toBe(1_000_000);
    });
    test("learn-up-only: never shrinks within a session", () => {
        expect(learnContextWindow(1_000_000, 50_000, "claude-sonnet-4-6")).toBe(1_000_000);
    });
    test("unknown model with no prior → undefined (provider fallback)", () => {
        expect(learnContextWindow(null, 50_000, "gpt-5")).toBeUndefined();
    });
    test("unknown model but a prior window → keeps the prior", () => {
        expect(learnContextWindow(200_000, 50_000, "gpt-5")).toBe(200_000);
    });
    test("model switch mid-session re-seeds (Opus 1M → Haiku 200K, not learn-up)", () => {
        // learned 1M on Opus, then /model to Haiku: must drop to 200K, not stay 1M
        expect(learnContextWindow(1_000_000, 50_000, "claude-haiku-4-5", "claude-opus-4-8")).toBe(200_000);
    });
    test("same model (unchanged) keeps the learned high-water", () => {
        expect(learnContextWindow(1_000_000, 50_000, "claude-sonnet-4-6", "claude-sonnet-4-6")).toBe(1_000_000);
    });
    test("model absent on a later turn keeps the prior window", () => {
        expect(learnContextWindow(1_000_000, 50_000, undefined, "claude-opus-4-8")).toBe(1_000_000);
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
