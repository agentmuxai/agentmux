// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, test } from "vitest";
import {
    contextReadingFromMeta,
    contextReadingNote,
    implausibleReason,
    learnedWindowsAfter,
    makeContextReading,
    mergeContextWindows,
    plausibleReading,
    refutedReportedWindow,
    reportedContextWindowsFromResult,
    reportedWindowFor,
    resolveContextWindow,
    rewindowContextReading,
    sameContextReading,
    windowFor,
    type ContextReading,
    type KnownContextWindows,
} from "./context-reading";

const reading = (over: Partial<ContextReading> = {}): ContextReading => ({
    tokens: 43_623,
    model: "claude-sonnet-5-5",
    window: 1_000_000,
    windowSource: "reported",
    source: "live",
    at: 1_000,
    switchedTo: null,
    ...over,
});

const known = (reported: Record<string, number> = {}, learned: Record<string, number> = {}): KnownContextWindows => ({
    reported,
    learned,
});

// The `result` frame of a real three-call turn on CLI 2.1.288 (identifiers
// trimmed): `usage` is the three calls' prompts summed (43,376 + 43,500 +
// 43,623 = 130,499 input), the context is the last call's 43,623.
const RESULT_FRAME = {
    type: "result",
    subtype: "success",
    num_turns: 3,
    usage: { input_tokens: 6, cache_creation_input_tokens: 31_614, cache_read_input_tokens: 98_879, output_tokens: 36 },
    modelUsage: {
        "claude-sonnet-5-5": {
            inputTokens: 6,
            outputTokens: 36,
            cacheReadInputTokens: 98_879,
            cacheCreationInputTokens: 31_614,
            webSearchRequests: 0,
            costUSD: 0.13,
            contextWindow: 1_000_000,
            maxOutputTokens: 128_000,
            canonicalModel: "claude-sonnet-5-5",
            provider: "firstParty",
        },
    },
};

describe("reportedContextWindowsFromResult", () => {
    test("reads modelUsage[*].contextWindow from a result frame", () => {
        expect(reportedContextWindowsFromResult(RESULT_FRAME)).toEqual({ "claude-sonnet-5-5": 1_000_000 });
    });

    test("records every model the turn used (a subagent's too) and a differing canonical id", () => {
        const frame = {
            type: "result",
            modelUsage: {
                "us.anthropic.claude-sonnet-5-5": { contextWindow: 1_000_000, canonicalModel: "claude-sonnet-5-5" },
                "claude-haiku-4-5-20251001": { contextWindow: 200_000 },
            },
        };
        expect(reportedContextWindowsFromResult(frame)).toEqual({
            "us.anthropic.claude-sonnet-5-5": 1_000_000,
            "claude-sonnet-5-5": 1_000_000,
            "claude-haiku-4-5-20251001": 200_000,
        });
    });

    test("ignores anything that isn't a believable window", () => {
        const frame = {
            type: "result",
            modelUsage: {
                a: { contextWindow: 0 },
                b: { contextWindow: -5 },
                c: { contextWindow: 1.5 },
                d: { contextWindow: "1000000" },
                e: { contextWindow: 500 },
                f: { contextWindow: 1e12 },
                g: null,
                h: {},
            },
        };
        expect(reportedContextWindowsFromResult(frame)).toBeNull();
    });

    test("null for anything but a result frame, or one with no modelUsage", () => {
        expect(reportedContextWindowsFromResult({ ...RESULT_FRAME, type: "assistant" })).toBeNull();
        expect(reportedContextWindowsFromResult({ type: "result" })).toBeNull();
        expect(reportedContextWindowsFromResult({ type: "result", modelUsage: {} })).toBeNull();
        expect(reportedContextWindowsFromResult(null)).toBeNull();
        expect(reportedContextWindowsFromResult("result")).toBeNull();
    });

    test("a [1m] main agent and a plain subagent of the same model: the larger window wins on the shared id", () => {
        // modelUsage is keyed by the raw model string; the API's message.model
        // (what a reading is measured on) carries no [1m] suffix.
        const frame = {
            type: "result",
            modelUsage: {
                "claude-sonnet-4-6[1m]": { contextWindow: 1_000_000, canonicalModel: "claude-sonnet-4-6" },
                "claude-sonnet-4-6": { contextWindow: 200_000, canonicalModel: "claude-sonnet-4-6" },
            },
        };
        expect(reportedContextWindowsFromResult(frame)).toEqual({
            "claude-sonnet-4-6[1m]": 1_000_000,
            "claude-sonnet-4-6": 1_000_000,
        });
        // Either order.
        const reversed = { type: "result", modelUsage: Object.fromEntries(Object.entries(frame.modelUsage).reverse()) };
        expect(reportedContextWindowsFromResult(reversed)?.["claude-sonnet-4-6"]).toBe(1_000_000);
    });

    test("a [1m] key with no canonicalModel is recorded under the bare id too", () => {
        expect(
            reportedContextWindowsFromResult({ type: "result", modelUsage: { "claude-opus-4-8[1m]": { contextWindow: 1_000_000 } } }),
        ).toEqual({ "claude-opus-4-8[1m]": 1_000_000, "claude-opus-4-8": 1_000_000 });
    });
});

describe("mergeContextWindows", () => {
    test("newer entries win; the base object comes back when nothing changes", () => {
        const base = { a: 200_000 };
        expect(mergeContextWindows(base, { a: 200_000 })).toBe(base);
        expect(mergeContextWindows(base, null)).toBe(base);
        expect(mergeContextWindows(base, { a: 1_000_000, b: 5_000 })).toEqual({ a: 1_000_000, b: 5_000 });
    });
});

describe("windowFor / reportedWindowFor", () => {
    test("exact id, then case-insensitive, then without a [1m] suffix; nothing for an unknown or absent model", () => {
        const reported = { "claude-sonnet-5-5": 1_000_000 };
        expect(reportedWindowFor(reported, "claude-sonnet-5-5")).toBe(1_000_000);
        expect(reportedWindowFor(reported, "CLAUDE-SONNET-5-5")).toBe(1_000_000);
        expect(windowFor(reported, "claude-sonnet-5-5[1m]")).toBe(1_000_000);
        expect(reportedWindowFor(reported, "claude-opus-5-5")).toBeUndefined();
        expect(reportedWindowFor(reported, null)).toBeUndefined();
    });
});

describe("resolveContextWindow", () => {
    test("a reported window outranks the table", () => {
        expect(resolveContextWindow(1, "claude-sonnet-4-6", known({ "claude-sonnet-4-6": 1_000_000 }))).toEqual({
            window: 1_000_000,
            windowSource: "reported",
        });
    });

    test("the table seeds; a larger prompt proves the next tier", () => {
        expect(resolveContextWindow(50_000, "claude-sonnet-4-6", known())).toEqual({ window: 200_000, windowSource: "model" });
        expect(resolveContextWindow(250_000, "claude-sonnet-4-6", known())).toEqual({ window: 1_000_000, windowSource: "learned" });
    });

    test("a learned window holds for its model, not for another", () => {
        const k = known({}, { "claude-sonnet-4-6": 1_000_000 });
        expect(resolveContextWindow(10_000, "claude-sonnet-4-6", k)).toEqual({ window: 1_000_000, windowSource: "learned" });
        expect(resolveContextWindow(10_000, "claude-haiku-4-5", k)).toEqual({ window: 200_000, windowSource: "model" });
    });

    test("an accepted prompt larger than the reported window refutes it; a larger learned window outranks it", () => {
        const k = known({ "claude-sonnet-4-6": 200_000 });
        expect(resolveContextWindow(250_000, "claude-sonnet-4-6", k)).toEqual({ window: 1_000_000, windowSource: "learned" });
        const learned = known({ "claude-sonnet-4-6": 200_000 }, { "claude-sonnet-4-6": 1_000_000 });
        expect(resolveContextWindow(10_000, "claude-sonnet-4-6", learned)).toEqual({ window: 1_000_000, windowSource: "learned" });
        // A reported window at least as large as the learned one is the authority again.
        const caughtUp = known({ "claude-sonnet-4-6": 1_000_000 }, { "claude-sonnet-4-6": 1_000_000 });
        expect(resolveContextWindow(10_000, "claude-sonnet-4-6", caughtUp)).toEqual({ window: 1_000_000, windowSource: "reported" });
    });

    test("unknown model → unknown window, whatever the prompt", () => {
        expect(resolveContextWindow(10_000, "gpt-5", known())).toEqual({ window: null, windowSource: null });
        expect(resolveContextWindow(500_000, "gpt-5", known())).toEqual({ window: null, windowSource: null });
        expect(resolveContextWindow(10_000, null, known())).toEqual({ window: null, windowSource: null });
    });

    test("only a model the table knows learns: the tiers are Claude's", () => {
        // A larger prompt on another model leaves its reported window, and the
        // reading is refused rather than promoted to a Claude tier.
        expect(resolveContextWindow(500_000, "gpt-5", known({ "gpt-5": 400_000 }))).toEqual({
            window: 400_000,
            windowSource: "reported",
        });
    });

    test("a prompt above every tier proves nothing — it is not a bigger window", () => {
        expect(resolveContextWindow(17_000_000, "claude-sonnet-5-5", known())).toEqual({ window: 1_000_000, windowSource: "model" });
        expect(resolveContextWindow(17_000_000, "claude-sonnet-5-5", known({ "claude-sonnet-5-5": 1_000_000 }))).toEqual({
            window: 1_000_000,
            windowSource: "reported",
        });
    });
});

describe("learnedWindowsAfter / refutedReportedWindow", () => {
    test("records what a reading proved, only upward, and nothing else", () => {
        const proved = reading({ model: "claude-sonnet-4-6", window: 1_000_000, windowSource: "learned" });
        expect(learnedWindowsAfter({}, proved)).toEqual({ "claude-sonnet-4-6": 1_000_000 });
        const had = { "claude-sonnet-4-6": 1_000_000 };
        expect(learnedWindowsAfter(had, proved)).toBe(had);
        const none = {};
        expect(learnedWindowsAfter(none, reading())).toBe(none);
        expect(learnedWindowsAfter(none, null)).toBe(none);
        expect(learnedWindowsAfter(none, { ...proved, model: null })).toBe(none);
    });

    test("names the reported window a learned one contradicts", () => {
        const proved = reading({ model: "claude-sonnet-4-6", window: 1_000_000, windowSource: "learned" });
        expect(refutedReportedWindow(proved, { "claude-sonnet-4-6": 200_000 })).toBe(200_000);
        expect(refutedReportedWindow(proved, {})).toBeNull();
        expect(refutedReportedWindow(reading(), { "claude-sonnet-5-5": 200_000 })).toBeNull();
    });
});

describe("makeContextReading / rewindowContextReading", () => {
    test("builds a reading with its window and provenance", () => {
        expect(makeContextReading({ tokens: 5, model: "claude-opus-5-5", source: "history", at: null }, known())).toEqual({
            tokens: 5,
            model: "claude-opus-5-5",
            window: 1_000_000,
            windowSource: "model",
            source: "history",
            at: null,
            switchedTo: null,
        });
    });

    test("re-windows to a reported value, and returns the same object when nothing changes", () => {
        const r = reading({ window: 200_000, windowSource: "model", model: "claude-sonnet-4-6" });
        expect(rewindowContextReading(r, known({ "claude-sonnet-4-6": 1_000_000 }))).toEqual({
            ...r,
            window: 1_000_000,
            windowSource: "reported",
        });
        expect(rewindowContextReading(r, known({ other: 1_000 }))).toBe(r);
        const reported = reading();
        expect(rewindowContextReading(reported, known({ "claude-sonnet-5-5": 1_000_000 }))).toBe(reported);
        expect(rewindowContextReading(null, known())).toBeNull();
    });

    test("re-windowing a reading that turns out larger than a newly reported window keeps it plausible", () => {
        // 250K measured on Sonnet 4.6, then the CLI reports 200K: the accepted
        // prompt wins and the reading stays on screen.
        const r = reading({ tokens: 250_000, model: "claude-sonnet-4-6", window: 1_000_000, windowSource: "learned" });
        const next = rewindowContextReading(r, known({ "claude-sonnet-4-6": 200_000 }, { "claude-sonnet-4-6": 1_000_000 }));
        expect(next).toBe(r);
        expect(plausibleReading(next)).not.toBeNull();
    });

    test("leaves a reading taken before a model switch alone: no report about the old model applies", () => {
        const switched = reading({ window: null, windowSource: null, switchedTo: "haiku" });
        expect(rewindowContextReading(switched, known({ "claude-sonnet-5-5": 1_000_000 }))).toBe(switched);
    });
});

describe("implausibleReason / plausibleReading", () => {
    test("a reading within its window is shown", () => {
        expect(implausibleReason(reading())).toBeNull();
        expect(plausibleReading(reading({ tokens: 1_000_000 }))).not.toBeNull();
    });

    test("17m / 200k is refused — more tokens than the window", () => {
        const bad = reading({ tokens: 17_000_000, window: 200_000, windowSource: "model" });
        expect(implausibleReason(bad)).toBe("tokens exceed the window");
        expect(plausibleReading(bad)).toBeNull();
    });

    test("with no window, anything above every known window is refused", () => {
        expect(implausibleReason(reading({ tokens: 900_000, window: null, windowSource: null }))).toBeNull();
        expect(implausibleReason(reading({ tokens: 1_000_001, window: null, windowSource: null }))).toBe(
            "tokens exceed every known window",
        );
    });

    test("non-positive or fractional counts are refused", () => {
        expect(implausibleReason(reading({ tokens: 0 }))).not.toBeNull();
        expect(implausibleReason(reading({ tokens: -3 }))).not.toBeNull();
        expect(implausibleReason(reading({ tokens: 1.5 }))).not.toBeNull();
        expect(implausibleReason(reading({ tokens: Number.NaN }))).not.toBeNull();
        expect(implausibleReason(reading({ window: 0 }))).not.toBeNull();
    });

    test("null in, null out", () => {
        expect(plausibleReading(null)).toBeNull();
        expect(plausibleReading(undefined)).toBeNull();
    });
});

describe("contextReadingFromMeta", () => {
    test("round-trips a reading written to meta", () => {
        const r = reading();
        expect(contextReadingFromMeta(JSON.parse(JSON.stringify(r)))).toEqual(r);
    });

    test("refuses the legacy bare number, junk, and implausible readings", () => {
        expect(contextReadingFromMeta(17_000_000)).toBeNull();
        expect(contextReadingFromMeta(null)).toBeNull();
        expect(contextReadingFromMeta({ tokens: "5", source: "live" })).toBeNull();
        expect(contextReadingFromMeta({ tokens: 5, source: "elsewhere" })).toBeNull();
        expect(contextReadingFromMeta({ ...reading(), tokens: 17_000_000, window: 200_000 })).toBeNull();
    });

    test("drops a window source without a window, and unknown optional fields", () => {
        expect(contextReadingFromMeta({ tokens: 5, source: "live", windowSource: "reported", model: 7 })).toEqual({
            tokens: 5,
            model: null,
            window: null,
            windowSource: null,
            source: "live",
            at: null,
            switchedTo: null,
        });
    });
});

describe("contextReadingNote", () => {
    test("says where a non-live reading and a non-reported window come from", () => {
        expect(contextReadingNote(reading())).toBe("Window reported by the CLI for claude-sonnet-5-5.");
        expect(contextReadingNote(reading({ source: "history", windowSource: "model" }))).toBe(
            "From the conversation's last reply before this pane opened; updates with the next reply.\n" +
                "Window assumed from the model (claude-sonnet-5-5) until the CLI reports it.",
        );
        expect(contextReadingNote(reading({ windowSource: "learned" }))).toMatch(/^Window inferred for claude-sonnet-5-5: it accepted/);
        expect(contextReadingNote(reading({ model: null, window: null, windowSource: null }))).toBe(
            "Window not known for this model yet.",
        );
        expect(contextReadingNote(reading({ window: null, windowSource: null, switchedTo: "haiku" }))).toBe(
            "Model changed to haiku; its window shows with its first reply.",
        );
    });
});

describe("sameContextReading", () => {
    test("compares by value", () => {
        expect(sameContextReading(reading(), reading())).toBe(true);
        expect(sameContextReading(reading(), reading({ at: 2 }))).toBe(false);
        expect(sameContextReading(reading(), reading({ switchedTo: "haiku" }))).toBe(false);
        expect(sameContextReading(null, null)).toBe(true);
        expect(sameContextReading(reading(), null)).toBe(false);
    });
});
