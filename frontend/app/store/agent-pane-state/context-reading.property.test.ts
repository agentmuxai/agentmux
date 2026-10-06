// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The context reading's invariants over random command sequences: whatever
 * order live calls, history seeds, reported windows, invalidations,
 * compactions, model switches and turn boundaries arrive in, the meter never
 * shows more tokens than its window, a history seed never produces a
 * "compacted" card, reported windows are never lost, and a seed never lands
 * after something more current has spoken.
 * docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md.
 */

import { describe, expect, it } from "vitest";

import { implausibleReason, plausibleReading, windowFor } from "./context-reading";
import { contextWindowForModel, MAX_KNOWN_CONTEXT_WINDOW } from "./context-window";
import { update } from "./reducer";
import { initialState, type AgentPaneCommand, type AgentPaneEvent, type AgentPaneState } from "./types";

/** mulberry32: a small seeded PRNG, so a failure names a reproducible seed. */
function rng(seed: number): () => number {
    let a = seed >>> 0;
    return () => {
        a = (a + 0x6d2b79f5) >>> 0;
        let t = a;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

const MODELS: (string | undefined)[] = [
    "claude-sonnet-4-6",
    "claude-sonnet-5-5",
    "claude-opus-4-8",
    "claude-opus-4-8[1m]",
    "claude-haiku-4-5",
    "gpt-5-codex",
    undefined,
];
const SETTINGS = ["sonnet", "opus", "haiku", "claude-sonnet-5-5"];
const WINDOWS = [128_000, 200_000, 1_000_000];
const pick = <T,>(r: () => number, xs: readonly T[]): T => xs[Math.floor(r() * xs.length)];

function tokens(r: () => number): number {
    switch (Math.floor(r() * 6)) {
        case 0:
            return 1 + Math.floor(r() * 10_000);
        case 1:
            return 10_000 + Math.floor(r() * 190_000);
        case 2:
            return 200_001 + Math.floor(r() * 799_999);
        case 3:
            return 1_000_001 + Math.floor(r() * 20_000_000);
        case 4:
            return 199_000 + Math.floor(r() * 2_000);
        default:
            return 30_000 + Math.floor(r() * 300_000);
    }
}

function windowsFor(r: () => number): Record<string, number> {
    const out: Record<string, number> = {};
    for (let i = 0, n = 1 + Math.floor(r() * 2); i < n; i++) {
        const m = pick(r, MODELS);
        if (m) out[r() < 0.2 ? m.toUpperCase() : m] = pick(r, WINDOWS);
    }
    return out;
}

function nextCommand(r: () => number, t: number): AgentPaneCommand {
    switch (Math.floor(r() * 12)) {
        case 0:
        case 1:
        case 2: {
            const model = pick(r, MODELS);
            return { type: "TokensIn", input: tokens(r), ...(model ? { model } : {}) };
        }
        case 3:
            return {
                type: "ReconcileContextFromHistory",
                tokens: r() < 0.8 ? tokens(r) : null,
                model: pick(r, MODELS) ?? null,
                at: r() < 0.5 ? t : null,
                reportedWindows: r() < 0.5 ? windowsFor(r) : null,
            };
        case 4:
            return { type: "ContextWindowsReported", windows: windowsFor(r) };
        case 5:
            return { type: "ContextInvalidated", reason: r() < 0.5 ? "fresh_session" : "transcript_replaced" };
        case 6: {
            const pre = tokens(r);
            return {
                type: "CompactionBoundary",
                trigger: r() < 0.5 ? "auto" : "manual",
                preTokens: pre,
                postTokens: Math.max(1, Math.floor(pre * r() * 0.3)),
                durationMs: 1000,
                at: t,
                frameTimestamp: r() < 0.5 ? new Date(t - Math.floor(r() * 5000)).toISOString() : null,
            };
        }
        case 7:
            return { type: "CompactionStarted", trigger: "auto", at: t };
        case 8:
            return { type: "TurnStart", at: t };
        case 9:
            return { type: "TurnEnd", stats: null };
        case 10:
            return { type: "ContextModelSwitched", model: pick(r, SETTINGS) };
        default:
            return r() < 0.5 ? { type: "TurnReset" } : { type: "TurnStartFailed" };
    }
}

function ready(): AgentPaneState {
    const s0 = update(initialState("prop-agent"), { type: "InitReady", at: 1_000 }).state;
    return update(s0, { type: "StreamSubscribe", at: 1_000 }).state;
}

function runOne(seed: number, steps: number): void {
    const r = rng(seed);
    let s = ready();
    let t = 1_000_000;
    const liveReported = new Map<string, number>();
    const everReported = new Set<string>();
    const trace: string[] = [];
    for (let i = 0; i < steps; i++) {
        t += Math.floor(r() * 120_000);
        const cmd = nextCommand(r, t);
        trace.push(JSON.stringify(cmd));
        const before = s;
        const { state, events } = update(s, cmd, t);
        s = state;
        const fail = (msg: string) => `seed=${seed} step=${i}: ${msg}\n` + trace.slice(-12).join("\n");

        if (cmd.type === "ContextWindowsReported") {
            for (const [k, w] of Object.entries(cmd.windows)) {
                liveReported.set(k, w);
                everReported.add(k);
            }
        }
        if (cmd.type === "ReconcileContextFromHistory" && cmd.reportedWindows) {
            for (const k of Object.keys(cmd.reportedWindows)) everReported.add(k);
        }

        // The meter never shows more tokens than its window.
        const shown = plausibleReading(s.context);
        if (shown) expect(shown.tokens, fail("tokens > window")).toBeLessThanOrEqual(shown.window ?? MAX_KNOWN_CONTEXT_WINDOW);

        // Window provenance is truthful.
        if (s.context && s.context.model && s.context.switchedTo == null) {
            const rep = windowFor(s.reportedContextWindows, s.context.model);
            if (s.context.windowSource === "reported") expect(s.context.window, fail("'reported' window != report")).toBe(rep);
            if (rep != null) expect(s.context.window, fail("window below the report")).toBeGreaterThanOrEqual(rep);
            if (s.context.windowSource === "model") {
                expect(s.context.window, fail("'model' window != table")).toBe(contextWindowForModel(s.context.model));
            }
            if (s.context.windowSource === "learned") {
                expect(contextWindowForModel(s.context.model), fail("learned on a model the table doesn't know")).not.toBeUndefined();
            }
        }
        if (s.context?.switchedTo != null) expect(s.context.window, fail("switched reading kept a window")).toBeNull();

        // A heuristic "compacted" card only ever compares two live readings.
        const heuristic = events.filter(
            (e): e is Extract<AgentPaneEvent, { type: "context-compacted" }> =>
                e.type === "context-compacted" && e.source === "heuristic",
        );
        if (heuristic.length > 0) {
            const prev = plausibleReading(before.context);
            expect(prev, fail("heuristic with no plausible previous reading")).not.toBeNull();
            expect(prev!.source, fail("heuristic against a history seed")).toBe("live");
            expect(heuristic[0].tokensBefore, fail("tokensBefore != previous reading")).toBe(prev!.tokens);
        }

        // Reported windows are never lost, and history never overrides live.
        for (const k of everReported) expect(k in s.reportedContextWindows, fail(`reported window for ${k} lost`)).toBe(true);
        for (const [k, w] of liveReported) expect(s.reportedContextWindows[k], fail(`live window for ${k} overridden`)).toBe(w);

        // Learned windows only grow.
        for (const [k, w] of Object.entries(before.learnedContextWindows)) {
            expect(s.learnedContextWindows[k], fail(`learned window for ${k} shrank`)).toBeGreaterThanOrEqual(w);
        }

        switch (cmd.type) {
            case "ContextInvalidated":
            case "TurnReset":
                expect(s.context, fail(`${cmd.type} left a reading`)).toBeNull();
                expect(s.contextSeedable, fail(`${cmd.type} left the pane seedable`)).toBe(false);
                break;
            case "TurnStartFailed":
            case "TurnStart":
            case "TurnEnd":
            case "CompactionStarted":
                expect(s.context, fail(`${cmd.type} changed the reading`)).toBe(before.context);
                break;
            case "CompactionBoundary":
                expect(s.contextSeedable, fail("a compaction left the pane seedable")).toBe(false);
                if (s.context) expect(s.context, fail("a boundary made up a reading")).toBe(before.context);
                break;
            case "ReconcileContextFromHistory":
                if (!before.contextSeedable || cmd.tokens == null) {
                    expect(s.context?.tokens, fail("a history seed landed late")).toBe(before.context?.tokens);
                    expect(s.context?.source, fail("a history seed landed late")).toBe(before.context?.source);
                } else if (s.context) {
                    expect(s.context.source).toBe("history");
                    expect(implausibleReason(s.context), fail("implausible history seed stored")).toBeNull();
                }
                break;
            case "TokensIn": {
                expect(s.context?.tokens).toBe(cmd.input);
                expect(s.context?.source).toBe("live");
                expect(s.contextSeedable).toBe(false);
                const rejected = events.some((e) => e.type === "context-reading-rejected");
                const implausible = implausibleReason(s.context!) != null;
                if (rejected) expect(implausible, fail("rejected a plausible reading")).toBe(true);
                if (implausible && (before.context == null || plausibleReading(before.context) != null)) {
                    expect(rejected, fail("an implausible reading went unreported")).toBe(true);
                }
                break;
            }
        }
    }
}

describe("context reading — properties over random command sequences", () => {
    // ~30k transitions; the timeout covers a loaded full-suite run.
    it("holds its invariants", { timeout: 60_000 }, () => {
        for (let seed = 1; seed <= 200; seed++) runOne(seed, 150);
    });
});
