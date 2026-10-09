// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane reducer: tokens, context reading and compaction.

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { turnOutputTokens } from "./turn-contribution";
import { plausibleReading } from "./context-reading";
import { AgentPaneState, INTERRUPT_TIMEOUT_MS, LIVENESS_RECOVERY_MS, STUCK_THRESHOLD_MS, SUBMIT_TIMEOUT_MS } from "./types";
import { streaming, mk, liveReading, ready } from "./reducer-test-fixtures";

describe("agent-pane-state reducer", () => {
    describe("Tokens", () => {
        it("TokensIn / TokensOut accumulate independently", () => {
            const s0 = update(mk(), { type: "TokensIn", input: 50 }).state;
            const s1 = update(s0, { type: "TokensOut", output: 100 }).state;
            expect(s1.turnTokens).toMatchObject({ input: 50, output: 100 });
        });

        it("a new call's TokensIn keeps the output so far in the turn's total", () => {
            const s0 = update(mk(), { type: "TokensOut", output: 100 }).state;
            const s1 = update(s0, { type: "TokensIn", input: 50 }).state;
            expect(s1.turnTokens).toMatchObject({ input: 50, output: 0, outputDone: 100 });
            expect(turnOutputTokens(s1.turnTokens)).toBe(100);
        });
    });

    // SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md — one
    // number, the turn's output, growing as it streams: ↑ while a request is in
    // flight, ↓ otherwise. Input is the context re-sent on every call, so it
    // isn't counted.
    describe("Tokens: the turn's output counter", () => {
        const call = (s: AgentPaneState, input: number) => update(s, { type: "TokensIn", input }).state;
        const out = (s: AgentPaneState, output: number) => update(s, { type: "TokensOut", output }).state;
        const streamed = (s: AgentPaneState, chars: number) => update(s, { type: "OutputStreamed", chars }).state;

        it("adds up the exact output of every call in the turn", () => {
            let s = call(mk(), 40_000);
            s = out(s, 300);
            s = call(s, 41_000);
            s = out(s, 200);
            expect(turnOutputTokens(s.turnTokens)).toBe(500);
        });

        it("counts a call still streaming at four characters a token until its exact count arrives", () => {
            let s = call(mk(), 40_000);
            s = streamed(s, 400);
            expect(turnOutputTokens(s.turnTokens)).toBe(100);
            s = out(s, 120);
            expect(turnOutputTokens(s.turnTokens)).toBe(120);
        });

        it("a call's repeated (cumulative) output counts once", () => {
            let s = call(mk(), 40_000);
            s = out(s, 50);
            s = out(s, 120);
            expect(turnOutputTokens(s.turnTokens)).toBe(120);
        });

        it("never goes down, even when a call's exact count is under its estimate", () => {
            let s = call(mk(), 40_000);
            s = streamed(s, 800); // estimated 200
            s = out(s, 150);
            expect(turnOutputTokens(s.turnTokens)).toBe(200);
            s = call(s, 41_000); // the first call is final at 150
            s = streamed(s, 200); // 150 + 50
            expect(turnOutputTokens(s.turnTokens)).toBe(200);
            s = out(s, 100);
            expect(turnOutputTokens(s.turnTokens)).toBe(250);
        });

        it("shows ↑ from a tool result to the next call, and ↓ once that call streams", () => {
            let s = call(mk(), 40_000);
            expect(s.turnTokens?.requesting).toBe(false);
            s = update(s, { type: "RequestStarted" }).state;
            expect(s.turnTokens?.requesting).toBe(true);
            s = call(s, 41_000);
            expect(s.turnTokens?.requesting).toBe(false);
        });

        it("streamed output also ends the request phase", () => {
            let s = update(call(mk(), 40_000), { type: "RequestStarted" }).state;
            s = streamed(s, 40);
            expect(s.turnTokens?.requesting).toBe(false);
        });

        it("before the turn's first call there is nothing to count", () => {
            const s0 = mk();
            expect(update(s0, { type: "OutputStreamed", chars: 100 }).state).toBe(s0);
            expect(update(s0, { type: "RequestStarted" }).state).toBe(s0);
        });

        it("TurnEnd keeps the result's output: the CLI's exact figure for the whole turn", () => {
            let s = call(streaming(110), 41_000);
            s = streamed(s, 2_000);
            const r = update(s, { type: "TurnEnd", stats: { input_tokens: 84_000, output_tokens: 512 } as any });
            expect(r.state.sessionStats).toMatchObject({ input_tokens: 84_000, output_tokens: 512 });
        });

        it("a result with no usage falls back to the live total", () => {
            let s = call(streaming(110), 41_000);
            s = out(s, 300);
            s = call(s, 42_000);
            s = out(s, 200);
            const r = update(s, { type: "TurnEnd", stats: { cost_usd: 0.01 } as any });
            expect(r.state.sessionStats?.output_tokens).toBe(500);
        });

        it("sessionTotals still add up the raw input (the cost/context accounting is unchanged)", () => {
            const s2 = call(streaming(110), 41_000);
            const r = update(s2, { type: "TurnEnd", stats: { input_tokens: 82_000, output_tokens: 10 } as any });
            expect(r.state.sessionTotals?.input_tokens).toBe(82_000);
        });

        // srv publishes `turn_active: false` when it reads the CLI's `result`
        // line, before it forwards that line to the pane, so the reconcile
        // lands while the stream is still delivering the turn. It used to
        // clear turnTokens, so TurnEnd found no live tokens to fall back on.
        describe("when the turn-ended push arrives before session_end", () => {
            const midTurn = () => {
                let s = call(streaming(110), 41_000);
                s = out(s, 300);
                return call(s, 43_000);
            };
            const reconciled = () => update(midTurn(), { type: "ReconcileTurnActive", at: 200, active: false }).state;
            // A result with no usage of its own, so TurnEnd shows what it merged.
            const noUsage = { cost_usd: 0.01 } as any;

            it("the turn's live tokens are still there for its TurnEnd", () => {
                const s = reconciled();
                expect(s.turnPhase.kind).toBe("Idle");
                const r = update(s, { type: "TurnEnd", stats: noUsage });
                expect(r.state.sessionStats?.output_tokens).toBe(300);
                expect(r.state.turnTokens).toBeNull();
            });

            it("so are they after the liveness recovery", () => {
                const s = midTurn();
                const recovered = update(s, {
                    type: "StreamWatchdogTick",
                    nowMs: (s.lastEventMs ?? 0) + LIVENESS_RECOVERY_MS + 1,
                }).state;
                expect(recovered.turnPhase.kind).toBe("Idle");
                const r = update(recovered, { type: "TurnEnd", stats: noUsage });
                expect(r.state.sessionStats?.output_tokens).toBe(300);
            });

            // The turn's last text is flushed right before its session_end
            // (useAgentStream), which re-promotes Idle to Streaming: that
            // promotion is the same turn, and must keep its tokens.
            it("the final-text flush before session_end keeps the turn's tokens", () => {
                const flushed = update(reconciled(), { type: "StreamFlushObserved", addedCount: 1, at: 250 }).state;
                expect(flushed.turnPhase.kind).toBe("Streaming");
                const r = update(flushed, { type: "TurnEnd", stats: noUsage });
                expect(r.state.sessionStats?.output_tokens).toBe(300);
            });

            it("a call of the same turn that the stream delivers after the push still counts toward it", () => {
                const late = out(call(reconciled(), 45_000), 200);
                const r = update(late, { type: "TurnEnd", stats: noUsage });
                expect(r.state.sessionStats?.output_tokens).toBe(500);
            });

            // The Idle phase the push leaves releases queued messages at once,
            // so the next turn can start before this turn's session_end.
            it("a queued message starting the next turn before session_end keeps this turn's tokens for it", () => {
                const started = update(reconciled(), { type: "TurnStart", at: 300 }).state;
                expect(started.turnPhase.kind).toBe("Submitting");
                const r = update(started, { type: "TurnEnd", stats: noUsage });
                expect(r.state.sessionStats?.output_tokens).toBe(300);
            });

            it("so does the backend's turn-active push for the next turn", () => {
                const promoted = update(reconciled(), { type: "ReconcileTurnActive", at: 300, active: true }).state;
                expect(promoted.turnPhase.kind).toBe("Streaming");
                const r = update(promoted, { type: "TurnEnd", stats: noUsage });
                expect(r.state.sessionStats?.output_tokens).toBe(300);
            });

            it("a message accepted mid-turn keeps the running turn's tokens", () => {
                const s = midTurn();
                const steered = update(s, { type: "TurnStart", at: 150 }).state;
                expect(steered.turnTokens).toBe(s.turnTokens);
            });

            // A turn whose process died before its `result` must not lend its
            // tokens to the next turn: the new session's `init` comes first in
            // the stream.
            it("a new CLI session in the stream drops tokens left by a turn that never got its result", () => {
                const s = update(reconciled(), { type: "StreamSessionStarted" }).state;
                expect(s.turnTokens).toBeNull();
                // The next session's first call starts the count from nothing.
                const next = update(s, { type: "TokensIn", input: 44_000 }).state;
                expect(turnOutputTokens(next.turnTokens)).toBe(0);
            });

            it("a new CLI session with nothing held changes nothing", () => {
                const s0 = streaming(110);
                expect(update(s0, { type: "StreamSessionStarted" }).state).toBe(s0);
            });
        });
    });

    // docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md — one
    // reading with its provenance; the window reported by the CLI wins.
    describe("Context reading", () => {
        it("a live call on Sonnet 5.5 reads 1M from the model table before anything is reported", () => {
            const s = update(mk(), { type: "TokensIn", input: 120_000, model: "claude-sonnet-5-5" }, 500).state;
            expect(s.context).toEqual({
                tokens: 120_000,
                model: "claude-sonnet-5-5",
                window: 1_000_000,
                windowSource: "model",
                source: "live",
                at: 500,
                switchedTo: null,
            });
            expect(s.lastContextModel).toBe("claude-sonnet-5-5");
        });

        it("a window the CLI reports replaces the table's, for the current reading and later ones", () => {
            let s = update(mk(), { type: "TokensIn", input: 120_000, model: "claude-sonnet-4-6" }).state;
            expect(s.context).toMatchObject({ window: 200_000, windowSource: "model" });
            s = update(s, { type: "ContextWindowsReported", windows: { "claude-sonnet-4-6": 1_000_000 } }).state;
            expect(s.context).toMatchObject({ window: 1_000_000, windowSource: "reported" });
            s = update(s, { type: "TokensIn", input: 130_000, model: "claude-sonnet-4-6" }).state;
            expect(s.context).toMatchObject({ tokens: 130_000, window: 1_000_000, windowSource: "reported" });
        });

        it("a subagent model's reported window doesn't touch a reading on another model", () => {
            let s = update(mk(), { type: "TokensIn", input: 120_000, model: "claude-sonnet-5-5" }).state;
            const before = s.context;
            s = update(s, { type: "ContextWindowsReported", windows: { "claude-haiku-4-5": 200_000 } }).state;
            expect(s.context).toBe(before);
            expect(s.reportedContextWindows).toEqual({ "claude-haiku-4-5": 200_000 });
        });

        it("re-reporting the same windows is a no-op (same state)", () => {
            const s = update(mk(), { type: "ContextWindowsReported", windows: { "claude-sonnet-5-5": 1_000_000 } }).state;
            const r = update(s, { type: "ContextWindowsReported", windows: { "claude-sonnet-5-5": 1_000_000 } });
            expect(r.state).toBe(s);
        });

        it("Sonnet 4.x learns 1M from a prompt above 200K, and keeps it on the same model", () => {
            let s = update(mk(), { type: "TokensIn", input: 250_000, model: "claude-sonnet-4-6" }).state;
            expect(s.context).toMatchObject({ window: 1_000_000, windowSource: "learned" });
            s = update(s, { type: "TokensIn", input: 20_000, model: "claude-sonnet-4-6" }).state;
            expect(s.context).toMatchObject({ window: 1_000_000, windowSource: "learned" });
        });

        it("a model switch re-resolves the window (Opus 1M → Haiku 200K)", () => {
            let s = update(mk(), { type: "TokensIn", input: 300_000, model: "claude-opus-5-5" }).state;
            s = update(s, { type: "TokensIn", input: 50_000, model: "claude-haiku-4-5" }).state;
            expect(s.context).toMatchObject({ window: 200_000, windowSource: "model", model: "claude-haiku-4-5" });
        });

        it("an unknown model has an unknown window, never a provider constant", () => {
            const s = update(mk(), { type: "TokensIn", input: 50_000, model: "some-future-model" }).state;
            expect(s.context).toMatchObject({ window: null, windowSource: null });
        });

        it("a call that omits the model is measured on the last one seen", () => {
            let s = update(mk(), { type: "TokensIn", input: 50_000, model: "claude-sonnet-5-5" }).state;
            s = update(s, { type: "TokensIn", input: 51_000 }).state;
            expect(s.context).toMatchObject({ model: "claude-sonnet-5-5", window: 1_000_000 });
        });

        it("a live reading that can't be true is kept but reported once per dispatch, and is no baseline", () => {
            const r = update(mk(), { type: "TokensIn", input: 2_000_000, model: "claude-haiku-4-5" });
            expect(r.events).toContainEqual({
                type: "context-reading-rejected",
                tokens: 2_000_000,
                window: 200_000,
                model: "claude-haiku-4-5",
                source: "live",
                reason: "tokens exceed the window",
            });
            const next = update(r.state, { type: "TokensIn", input: 10_000, model: "claude-haiku-4-5" });
            expect(next.events.filter((e) => e.type === "context-compacted")).toEqual([]);
        });

        it("ContextInvalidated clears the reading (a fresh session replaced the conversation)", () => {
            const s = update(mk(), { type: "TokensIn", input: 300_000, model: "claude-sonnet-5-5" }).state;
            const r = update(s, { type: "ContextInvalidated", reason: "fresh_session" });
            expect(r.state.context).toBeNull();
            // ...so the fresh session's small first call is not a "compaction".
            const next = update(r.state, { type: "TokensIn", input: 20_000, model: "claude-sonnet-5-5" });
            expect(next.events.filter((e) => e.type === "context-compacted")).toEqual([]);
            expect(update(r.state, { type: "ContextInvalidated", reason: "fresh_session" }).state).toBe(r.state);
        });

        it("TurnReset clears the reading but keeps the reported windows", () => {
            let s = update(mk(), { type: "ContextWindowsReported", windows: { "claude-sonnet-5-5": 1_000_000 } }).state;
            s = update(s, { type: "TokensIn", input: 300_000, model: "claude-sonnet-5-5" }).state;
            s = update(s, { type: "TurnReset" }).state;
            expect(s.context).toBeNull();
            expect(s.reportedContextWindows).toEqual({ "claude-sonnet-5-5": 1_000_000 });
        });

        it("TurnStartFailed (a locally handled /command) keeps the reading and the session totals", () => {
            const s0 = ready(100);
            const s1 = update({ ...s0, context: liveReading(40_000), sessionTotals: { input_tokens: 9 } as any }, { type: "TurnStart", at: 110 }).state;
            const r = update(s1, { type: "TurnStartFailed" });
            expect(r.state.context?.tokens).toBe(40_000);
            expect(r.state.sessionTotals).toEqual({ input_tokens: 9 });
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("a compaction clears the reading until the next call: post_tokens is not the context size", () => {
            // CLI 2.1.288: a /compact reported post_tokens 1,417 (the summary
            // messages); the next call's prompt was 39,490 (system prompt and
            // tools included).
            let s = update(mk(), { type: "TokensIn", input: 40_673, model: "claude-sonnet-5-5" }, 40).state;
            s = update(s, { type: "CompactionBoundary", trigger: "manual", preTokens: 40_697, postTokens: 1_417, durationMs: 1, at: 50 }, 50).state;
            expect(s.context).toBeNull();
            const r = update(s, { type: "TokensIn", input: 39_490, model: "claude-sonnet-5-5" }, 60);
            expect(r.state.context).toMatchObject({ tokens: 39_490, window: 1_000_000, source: "live" });
            // No "before" to compare against, so no heuristic card on top of the real one.
            expect(r.events.some((e) => e.type === "context-compacted")).toBe(false);
        });

        it("a late boundary leaves a reading taken after it", () => {
            let s = update(mk(), { type: "TokensIn", input: 39_490, model: "claude-sonnet-5-5" }, 2_000).state;
            s = update(
                s,
                {
                    type: "CompactionBoundary",
                    trigger: "auto",
                    preTokens: 900_000,
                    postTokens: 30_000,
                    durationMs: 1,
                    at: 2_100,
                    frameTimestamp: new Date(1_500).toISOString(),
                },
                2_100,
            ).state;
            expect(s.context).toMatchObject({ tokens: 39_490, source: "live" });
        });

        it("a history seed can't bring back a conversation invalidated while history was loading", () => {
            // The live `fresh` outcome is drained before the restored window
            // seeds (useHistoryPagination settles, then seeds).
            let s = update(mk(), { type: "ContextInvalidated", reason: "fresh_session" }).state;
            expect(s.contextSeedable).toBe(false);
            const r = update(s, { type: "ReconcileContextFromHistory", tokens: 300_000, model: "claude-sonnet-5-5" });
            expect(r.state.context).toBeNull();
            expect(r.events).toEqual([]);
            // Same after a reset or a compaction.
            s = update(mk(), { type: "TurnReset" }).state;
            expect(update(s, { type: "ReconcileContextFromHistory", tokens: 300_000 }).state.context).toBeNull();
            s = update(mk(), { type: "CompactionBoundary", trigger: "auto", preTokens: 9, postTokens: 1, durationMs: 1, at: 5 }).state;
            expect(update(s, { type: "ReconcileContextFromHistory", tokens: 300_000 }).state.context).toBeNull();
        });

        it("a seed lands once; a second restore doesn't replace it", () => {
            let s = update(mk(), { type: "ReconcileContextFromHistory", tokens: 300_000, model: "claude-sonnet-5-5" }).state;
            s = update(s, { type: "ReconcileContextFromHistory", tokens: 5_000, model: "claude-sonnet-5-5" }).state;
            expect(s.context?.tokens).toBe(300_000);
        });

        it("history windows merge under live ones, with or without a reading to seed", () => {
            let s = update(mk(), { type: "ContextWindowsReported", windows: { "claude-sonnet-4-6": 1_000_000 } }).state;
            s = update(s, {
                type: "ReconcileContextFromHistory",
                tokens: null,
                reportedWindows: { "claude-sonnet-4-6": 200_000, "claude-haiku-4-5": 200_000 },
            }).state;
            expect(s.reportedContextWindows).toEqual({ "claude-sonnet-4-6": 1_000_000, "claude-haiku-4-5": 200_000 });
            expect(s.context).toBeNull();
            // Still seedable: a window-only restore measured nothing.
            expect(s.contextSeedable).toBe(true);
        });

        it("an accepted prompt larger than the reported window refutes it, once, and the reading stays shown", () => {
            let s = update(mk(), { type: "ContextWindowsReported", windows: { "claude-sonnet-4-6": 200_000 } }).state;
            let r = update(s, { type: "TokensIn", input: 250_000, model: "claude-sonnet-4-6" });
            expect(r.state.context).toMatchObject({ tokens: 250_000, window: 1_000_000, windowSource: "learned" });
            expect(plausibleReading(r.state.context)).not.toBeNull();
            expect(r.events).toContainEqual({
                type: "context-window-refuted",
                model: "claude-sonnet-4-6",
                reported: 200_000,
                tokens: 250_000,
                window: 1_000_000,
            });
            s = r.state;
            r = update(s, { type: "TokensIn", input: 260_000, model: "claude-sonnet-4-6" });
            expect(r.events.some((e) => e.type === "context-window-refuted")).toBe(false);
            // A small prompt later keeps the proven window.
            r = update(r.state, { type: "TokensIn", input: 20_000, model: "claude-sonnet-4-6" });
            expect(r.state.context).toMatchObject({ window: 1_000_000, windowSource: "learned" });
        });

        it("a learned window outlives TurnReset (B7)", () => {
            let s = update(mk(), { type: "TokensIn", input: 400_000, model: "claude-sonnet-4-6" }).state;
            expect(s.context?.window).toBe(1_000_000);
            s = update(s, { type: "TurnReset" }).state;
            s = update(s, { type: "TokensIn", input: 150_000, model: "claude-sonnet-4-6" }).state;
            expect(s.context).toMatchObject({ tokens: 150_000, window: 1_000_000, windowSource: "learned" });
        });

        it("says so when a reading the meter showed turns implausible, once", () => {
            // A reading outside every tier with an unknown window is shown...
            let s = update(mk(), { type: "TokensIn", input: 900_000, model: "some-model" }).state;
            expect(plausibleReading(s.context)).not.toBeNull();
            // ...until the CLI reports a smaller window for it.
            const r = update(s, { type: "ContextWindowsReported", windows: { "some-model": 128_000 } });
            expect(plausibleReading(r.state.context)).toBeNull();
            expect(r.events).toContainEqual(expect.objectContaining({ type: "context-reading-rejected", reason: "tokens exceed the window" }));
            // A further call that stays impossible is not news.
            s = r.state;
            const again = update(s, { type: "TokensIn", input: 950_000, model: "some-model" });
            expect(again.events.some((e) => e.type === "context-reading-rejected")).toBe(false);
        });

        it("a model switch blanks the window until the new model replies, and keeps the tokens", () => {
            let s = update(mk(), { type: "TokensIn", input: 300_000, model: "claude-sonnet-5-5" }).state;
            s = update(s, { type: "ContextModelSwitched", model: "haiku" }).state;
            expect(s.context).toMatchObject({ tokens: 300_000, window: null, windowSource: null, switchedTo: "haiku" });
            // A late report about the old model doesn't bring its window back.
            s = update(s, { type: "ContextWindowsReported", windows: { "claude-sonnet-5-5": 1_000_000 } }).state;
            expect(s.context?.window).toBeNull();
            // The new model's first reply resolves its own window.
            s = update(s, { type: "TokensIn", input: 120_000, model: "claude-haiku-4-5" }).state;
            expect(s.context).toMatchObject({ tokens: 120_000, window: 200_000, switchedTo: null });
        });

        it("switching back to the reading's own model restores its window", () => {
            let s = update(mk(), { type: "TokensIn", input: 300_000, model: "claude-sonnet-5-5" }).state;
            s = update(s, { type: "ContextModelSwitched", model: "haiku" }).state;
            s = update(s, { type: "ContextModelSwitched", model: "claude-sonnet-5-5" }).state;
            expect(s.context).toMatchObject({ window: 1_000_000, windowSource: "model", switchedTo: null });
            // No reading, nothing to switch.
            expect(update(mk(), { type: "ContextModelSwitched", model: "haiku" }).state.context).toBeNull();
        });

        it("archive/restore invalidates like a fresh session", () => {
            const s = update(mk(), { type: "TokensIn", input: 300_000, model: "claude-sonnet-5-5" }).state;
            const r = update(s, { type: "ContextInvalidated", reason: "transcript_replaced" });
            expect(r.state.context).toBeNull();
            // The new conversation's first call isn't a "compaction".
            const next = update(r.state, { type: "TokensIn", input: 20_000, model: "claude-sonnet-5-5" });
            expect(next.events.some((e) => e.type === "context-compacted")).toBe(false);
        });
    });

    describe("Compaction (SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31)", () => {
        describe("CompactionStarted", () => {
            it("sets compacting and bumps lastEventMs while genuinely working (Streaming)", () => {
                const s0 = streaming(100);
                const r = update(s0, { type: "CompactionStarted", trigger: "manual", at: 200 }, 200);
                expect(r.state.compacting).toEqual({ trigger: "manual", startedAt: 200 });
                expect(r.state.lastEventMs).toBe(200);
                expect(r.events).toEqual([{ type: "compaction-started", trigger: "manual" }]);
            });

            it("is a no-op when the stream is not subscribed", () => {
                const s0 = mk();
                const r = update(s0, { type: "CompactionStarted", trigger: "auto", at: 200 });
                expect(r.state).toBe(s0);
                expect(r.events).toEqual([]);
            });

            it("buffers onto pendingCompactionPing when subscribed but Idle, instead of dropping (SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02)", () => {
                // `compaction_started` arrives over a SEPARATE transport
                // (MPS) from the primary NDJSON stream carrying TurnEnd /
                // compact_boundary AND from the ReconcileTurnActive RPC, so
                // it can race and land while the pane is Idle for either of
                // two very different reasons: (a) a real turn already ended
                // (round 5's original case — the ping is genuinely stale),
                // or (b) this pane just mounted/resumed and hasn't been
                // reconciled to Streaming YET even though a real turn IS
                // in flight (the bug this buffering fixes). The reducer
                // can't yet tell which — it buffers rather than setting
                // `compacting` directly (still refusing that — round 5's
                // orphan-state guard holds), and defers the decision to
                // whichever authoritative signal resolves the ambiguity
                // next: `ReconcileTurnActive` or `StreamFlushObserved`
                // promoting this same buffered ping, or `ReconcileTurnActive
                // (active: false)` / TurnStart discarding it. See the
                // "pendingCompactionPing" describe block below for those.
                const s0 = ready(100); // subscribed, but Idle — no TurnStart
                const r = update(s0, { type: "CompactionStarted", trigger: "manual", at: 200 }, 200);
                expect(r.state.compacting).toBeNull(); // still NOT set directly
                expect(r.state.pendingCompactionPing).toEqual({ trigger: "manual", startedAt: 200 });
                expect(r.events).toEqual([{ type: "compaction-started-buffered", trigger: "manual" }]);
            });

            it("buffers onto pendingCompactionPing once the turn has ended in Done.completed — a later round can still be the same turn", () => {
                // Done.completed specifically (not just any Done): a
                // multi-round tool-continuation fires session_end after
                // EVERY round, so a ping landing here can legitimately be
                // for the NEXT round of the same overall turn — the same
                // standard StreamFlushObserved's own promotion arm already
                // uses for Done.completed.
                const s0 = update(streaming(100), { type: "TurnEnd", stats: null }, 150).state;
                expect(s0.turnPhase).toEqual({ kind: "Done", outcome: "completed", finishedAt: 150 });
                const r = update(s0, { type: "CompactionStarted", trigger: "auto", at: 200 }, 200);
                expect(r.state.compacting).toBeNull();
                expect(r.state.pendingCompactionPing).toEqual({ trigger: "auto", startedAt: 200 });
            });

            it("is a true no-op for a genuinely terminal Done (errored/stopped/interrupted) — nothing can ever promote it again", () => {
                // Unlike Done.completed, these outcomes are NOT in
                // StreamFlushObserved's own promotion set either — buffering
                // here would strand pendingCompactionPing exactly the way
                // round 5 originally worried about for `compacting`.
                const s0 = update(streaming(100), { type: "FailureObserved", failure: {
                    code: "rate_limited", title: "t", detail: "d", retryable: true,
                }, at: 150 }).state;
                expect(s0.turnPhase).toEqual({ kind: "Done", outcome: "errored", finishedAt: 150 });
                const r = update(s0, { type: "CompactionStarted", trigger: "auto", at: 200 }, 200);
                expect(r.state).toBe(s0);
                expect(r.state.compacting).toBeNull();
                expect(r.state.pendingCompactionPing).toBeNull();
                expect(r.events).toEqual([]);
            });

            it("refreshes a Streaming phase's own lastEventMs so the watchdog doesn't misfire", () => {
                const s0 = streaming(100);
                const r = update(s0, { type: "CompactionStarted", trigger: "auto", at: 500 }, 500);
                expect(r.state.turnPhase.kind).toBe("Streaming");
                if (r.state.turnPhase.kind === "Streaming") {
                    expect(r.state.turnPhase.lastEventMs).toBe(500);
                }
            });

            it("is a no-op when the start's own timestamp is at or before the last known CompactionBoundary (reagent, round 6)", () => {
                // Narrower race than round 5's fix: compaction_started and
                // compact_boundary travel over two independent transports
                // (MPS vs. the primary NDJSON stream) with no ordering
                // guarantee. A stale start can arrive AFTER its own
                // matching boundary while the turn is STILL working (e.g.
                // streaming new content past the compaction that already
                // completed) -- workingFromPhase alone can't catch this,
                // since it stays true the whole time.
                const s0 = update(streaming(100), {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 150,
                }, 150).state;
                expect(s0.lastCompactionBoundaryAt).toBe(150);
                // Equal-to-boundary timestamp: still rejected (not strictly after).
                const rEqual = update(s0, { type: "CompactionStarted", trigger: "manual", at: 150 }, 200);
                expect(rEqual.state).toBe(s0);
                expect(rEqual.state.compacting).toBeNull();
                // Before the boundary: also rejected.
                const rBefore = update(s0, { type: "CompactionStarted", trigger: "manual", at: 120 }, 200);
                expect(rBefore.state.compacting).toBeNull();
            });

            it("accepts a genuinely new start whose timestamp is after the last known CompactionBoundary", () => {
                const s0 = update(streaming(100), {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 150,
                }, 150).state;
                const r = update(s0, { type: "CompactionStarted", trigger: "auto", at: 200 }, 200);
                expect(r.state.compacting).toEqual({ trigger: "auto", startedAt: 200 });
            });
        });

        describe("pendingCompactionPing promotion/discard (SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02)", () => {
            // Regression coverage for the "Working… disappears mid-compaction
            // after a load/resume" bug and the two follow-up races reviewers
            // found in earlier (hook-only) fix attempts on PR #2928: a retry
            // heuristic that only observes the RESULTING turnPhase value
            // cannot tell "this promotion is the SAME turn the buffered ping
            // was about" from "this is a later, unrelated turn," and cannot
            // react to an authoritative ReconcileTurnActive(active: false)
            // confirmation that happens not to change turnPhase at all. Both
            // gaps are closed here because the reducer sees the discrete
            // command itself, not just its effect on the resulting state.

            it("ReconcileTurnActive(active: true) promotes a buffered ping into compacting", () => {
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.pendingCompactionPing).toEqual({ trigger: "manual", startedAt: 150 });
                const r = update(s0, { type: "ReconcileTurnActive", at: 200, active: true });
                expect(r.state.turnPhase.kind).toBe("Streaming");
                expect(r.state.compacting).toEqual({ trigger: "manual", startedAt: 150 });
                expect(r.state.pendingCompactionPing).toBeNull();
                expect(r.events).toEqual([
                    { type: "turn-active-reconciled" },
                    { type: "compaction-started", trigger: "manual" },
                ]);
            });

            it("ReconcileTurnActive(active: true) with no buffered ping behaves exactly as before", () => {
                const start = mk();
                const r = update(start, { type: "ReconcileTurnActive", at: 100, active: true });
                expect(r.state.compacting).toBeNull();
                expect(r.events).toEqual([{ type: "turn-active-reconciled" }]);
            });

            it("ReconcileTurnActive(active: false) clears a buffered ping even though turnPhase stays Idle — a same-ref no-op for the phase alone (reagent P1 + codex P2 on PR #2928)", () => {
                // The exact gap in both prior hook-only attempts: this
                // authoritative "no turn is active" confirmation must clear
                // pendingCompactionPing REGARDLESS of whether turnPhase
                // itself changes (it doesn't here — Idle stays Idle) so a
                // later UNRELATED TurnStart can never inherit a stale ping.
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.turnPhase.kind).toBe("Idle");
                expect(s0.pendingCompactionPing).not.toBeNull();
                const r = update(s0, { type: "ReconcileTurnActive", at: 200, active: false });
                expect(r.state.turnPhase.kind).toBe("Idle"); // unchanged
                expect(r.state.pendingCompactionPing).toBeNull(); // but explicitly cleared
                expect(r.events).toEqual([{ type: "turn-inactive-reconciled", at: 200 }]);
            });

            it("ReconcileTurnActive(active: false) is still a true same-ref no-op when there was nothing buffered", () => {
                const start = mk();
                const r = update(start, { type: "ReconcileTurnActive", at: 100, active: false });
                expect(r.state).toBe(start);
                expect(r.events).toEqual([]);
            });

            it("StreamFlushObserved promotes a buffered ping into compacting (resumed live content proves the same turn is active)", () => {
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "auto", at: 150 }, 150).state;
                const r = update(s0, { type: "StreamFlushObserved", addedCount: 1, at: 200 });
                expect(r.state.turnPhase.kind).toBe("Streaming");
                expect(r.state.compacting).toEqual({ trigger: "auto", startedAt: 150 });
                expect(r.state.pendingCompactionPing).toBeNull();
                expect(r.events).toContainEqual({ type: "compaction-started", trigger: "auto" });
            });

            it("StreamFlushObserved while ALREADY Streaming does not consume a buffered ping that has nothing to do with a promotion", () => {
                // Guard against a broader-than-intended condition: only an
                // actual Idle/Disconnected/Done→Streaming PROMOTION consumes
                // the buffer, never a same-phase refresh.
                const s0 = streaming(100);
                // Not reachable via CompactionStarted while genuinely
                // Streaming (that path sets `compacting` directly) — force
                // the field to prove the guard, mirroring how other tests in
                // this file construct otherwise-unreachable intermediate
                // states to pin an invariant.
                const withPending: AgentPaneState = { ...s0, pendingCompactionPing: { trigger: "manual", startedAt: 50 } };
                const r = update(withPending, { type: "StreamFlushObserved", addedCount: 1, at: 200 });
                expect(r.state.pendingCompactionPing).toEqual({ trigger: "manual", startedAt: 50 });
                expect(r.events).not.toContainEqual(expect.objectContaining({ type: "compaction-started" }));
            });

            it("TurnStart discards (does not promote) a buffered ping — a fresh turn is unrelated to whatever it was about (codex P2 on PR #2928)", () => {
                // The exact scenario codex flagged against the earlier
                // hook-only fix: without this, the VERY NEXT
                // StreamFlushObserved (Submitting→Streaming, which fires for
                // practically every TurnStart) would otherwise inherit and
                // promote the stale ping onto this brand-new, unrelated turn.
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.pendingCompactionPing).not.toBeNull();
                const s1 = update(s0, { type: "TurnStart", at: 200 }).state;
                expect(s1.turnPhase.kind).toBe("Submitting");
                expect(s1.pendingCompactionPing).toBeNull();
                // And the natural next event (Submitting → Streaming) confirms
                // nothing was falsely inherited.
                const r = update(s1, { type: "StreamFlushObserved", addedCount: 1, at: 210 });
                expect(r.state.turnPhase.kind).toBe("Streaming");
                expect(r.state.compacting).toBeNull();
            });

            it("CompactionBoundary clears a buffered ping once its own compaction is confirmed already finished", () => {
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 300,
                    frameTimestamp: new Date(200).toISOString(),
                }, 300);
                expect(r.state.pendingCompactionPing).toBeNull();
            });

            it("CompactionBoundary preserves a NEWER buffered ping against an out-of-order delayed boundary for an older compaction", () => {
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "manual", at: 1_000 }, 1_000).state;
                expect(s0.pendingCompactionPing).toEqual({ trigger: "manual", startedAt: 1_000 });
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 1_500,
                    frameTimestamp: new Date(500).toISOString(), // predates the buffered ping's own start
                }, 1_500);
                expect(r.state.pendingCompactionPing).toEqual({ trigger: "manual", startedAt: 1_000 });
            });

            it("TurnEnd clears a buffered ping (whatever it was about is moot once the turn ends)", () => {
                const s0 = update(streaming(100), { type: "TurnEnd", stats: null }, 150).state;
                const s1 = update(s0, { type: "CompactionStarted", trigger: "auto", at: 160 }, 160).state;
                expect(s1.pendingCompactionPing).not.toBeNull();
                const r = update(s1, { type: "TurnEnd", stats: null }, 200);
                expect(r.state.pendingCompactionPing).toBeNull();
            });

            it("StreamUnsubscribe clears a buffered ping along with a mid-compaction disconnect", () => {
                const s0 = update(streaming(100), { type: "RequestStop", at: 150 }).state;
                const s1 = update(s0, { type: "StreamUnsubscribe", at: 200 });
                // wasWorking path (Interrupting → Disconnected) already
                // clears compacting; pendingCompactionPing can't coexist with
                // a working phase anyway, so this pins the field stays null
                // through the same transition rather than silently drifting.
                expect(s1.state.pendingCompactionPing).toBeNull();
            });

            it("StreamUnsubscribe from a non-working phase ALSO clears a buffered ping (reagent P1 on PR #2928, round 2)", () => {
                // The gap reagent found in the early same-ref no-op branch:
                // pendingCompactionPing CAN be set while Idle (that's exactly
                // the buffering condition), so unsubscribing (e.g. tab
                // backgrounded pre-reconciliation) must still discard it —
                // otherwise a later resubscribe + resumed content could
                // retroactively promote the stale ping onto an unrelated turn.
                const s0 = update(ready(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.turnPhase.kind).toBe("Idle");
                expect(s0.pendingCompactionPing).not.toBeNull();
                const r = update(s0, { type: "StreamUnsubscribe", at: 200 });
                expect(r.state.turnPhase.kind).toBe("Idle"); // unchanged — still not "working"
                expect(r.state.pendingCompactionPing).toBeNull();
            });

            it("StreamUnsubscribe from a non-working phase with NOTHING buffered is still a true same-ref no-op", () => {
                const s0 = ready(100);
                const r = update(s0, { type: "StreamUnsubscribe", at: 200 });
                expect(r.state).toBe(s0);
                expect(r.events).toEqual([]);
            });

            it("TurnEnd while Disconnected ALSO clears a buffered ping (reagent P1 on PR #2928, round 2)", () => {
                // Same gap as StreamUnsubscribe's early branch: a late TurnEnd
                // ack arriving while Disconnected (a promotable-later phase)
                // is a same-ref no-op for turnPhase, but is still an
                // authoritative "this turn genuinely ended" signal that must
                // discard any buffered ping.
                //
                // NOTE: `pendingCompactionPing` can't actually get buffered
                // via a live CompactionStarted dispatch while Disconnected
                // in today's system — StreamUnsubscribe always pairs
                // Disconnected with `lastEventMs: null`, and CompactionStarted
                // gates on `lastEventMs != null` before it ever reaches the
                // buffering branch, so real-world reachability goes through
                // StreamUnsubscribe's OWN early-branch fix instead (see the
                // tests above). This test pins the defense-in-depth guard
                // directly (mirrors the "StreamFlushObserved while ALREADY
                // Streaming" test's same construction technique below) in
                // case that invariant ever changes.
                const s0 = update(streaming(100), { type: "StreamUnsubscribe", at: 140 }).state;
                expect(s0.turnPhase.kind).toBe("Disconnected");
                const withPending: AgentPaneState = { ...s0, pendingCompactionPing: { trigger: "auto", startedAt: 150 } };
                const r = update(withPending, { type: "TurnEnd", stats: null }, 200);
                expect(r.state.turnPhase.kind).toBe("Disconnected"); // unchanged
                expect(r.state.pendingCompactionPing).toBeNull();
            });

            it("TurnEnd while Disconnected with NOTHING buffered is still a true same-ref no-op", () => {
                const s0 = update(streaming(100), { type: "StreamUnsubscribe", at: 140 }).state;
                const r = update(s0, { type: "TurnEnd", stats: null }, 200);
                expect(r.state).toBe(s0);
                expect(r.events).toEqual([]);
            });
        });

        describe("CompactionBoundary", () => {
            it("clears compacting, records lastCompactionBoundaryAt, and clears the context reading", () => {
                const s0 = update(streaming(100), {
                    type: "CompactionStarted",
                    trigger: "manual",
                    at: 200,
                }, 200).state;
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 100_000,
                    postTokens: 5_000,
                    durationMs: 12_000,
                    at: 300,
                }, 300);
                expect(r.state.compacting).toBeNull();
                expect(r.state.lastCompactionBoundaryAt).toBe(300);
                expect(r.state.context).toBeNull();
                expect(r.state.contextSeedable).toBe(false);
                expect(r.events).toEqual([
                    {
                        type: "context-compacted",
                        tokensBefore: 100_000,
                        tokensAfter: 5_000,
                        source: "real",
                        trigger: "manual",
                        durationMs: 12_000,
                    },
                ]);
            });

            it("works even if no CompactionStarted preceded it (compact_boundary without a live PreCompact hook signal)", () => {
                const r = update(mk(), {
                    type: "CompactionBoundary",
                    trigger: "auto",
                    preTokens: 50_000,
                    postTokens: 2_000,
                    durationMs: 8_000,
                    at: 100,
                });
                expect(r.state.compacting).toBeNull();
                expect(r.state.context).toBeNull();
                expect(r.events[0]).toMatchObject({ type: "context-compacted", source: "real", trigger: "auto" });
            });

            // codex P1 on PR #2659: a manual "/compact" (the composer's
            // "Compact now" button, or a user typing it) goes through
            // handleSendMessage, which dispatches TurnStart the same as any
            // real conversational turn — but the CLI's response to a
            // standalone /compact is never an assistant text frame or a
            // `result` frame, only `system/status` + this boundary. Without
            // ending the turn here, the pane is stuck "Working" forever
            // after every successful manual compaction.
            describe("ends a synthetic manual-compact turn (codex P1, PR #2659)", () => {
                /** Like `streaming()`, but the turn was started for "/compact"
                 *  specifically — sets `pendingCompactTurn`, the signal
                 *  CompactionBoundary actually gates on. */
                const compactStreaming = (atMs = 100) => {
                    const s1 = update(ready(atMs), { type: "TurnStart", at: atMs, content: "/compact" }).state;
                    return update(s1, { type: "StreamFlushObserved", addedCount: 1, at: atMs }).state;
                };

                it("transitions a Streaming turn to Done.completed on a MANUAL boundary, when the turn was started for /compact", () => {
                    const s0 = compactStreaming(100);
                    const r = update(s0, {
                        type: "CompactionBoundary",
                        trigger: "manual",
                        preTokens: 30_000,
                        postTokens: 2_000,
                        durationMs: 5_000,
                        at: 200,
                    }, 200);
                    expect(r.state.turnPhase).toEqual({ kind: "Done", outcome: "completed", finishedAt: 200 });
                    expect(r.state.currentTool).toBeNull();
                    expect(r.state.currentToolArg).toBeNull();
                    expect(r.state.turnTokens).toBeNull();
                    expect(r.state.pendingCompactTurn).toBe(false);
                    // The existing compaction bookkeeping still runs unchanged.
                    expect(r.state.compacting).toBeNull();
                    expect(r.state.context).toBeNull();
                });

                it("does NOT end an AUTO-triggered compaction's turn — auto-compaction happens transparently mid-turn", () => {
                    const s0 = compactStreaming(100);
                    const r = update(s0, {
                        type: "CompactionBoundary",
                        trigger: "auto",
                        preTokens: 30_000,
                        postTokens: 2_000,
                        durationMs: 5_000,
                        at: 200,
                    }, 200);
                    expect(r.state.turnPhase.kind).toBe("Streaming");
                });

                it("does NOT end an unrelated real turn on a MANUAL boundary, when THIS pane's turn wasn't started for /compact (reducer race-guard regression)", () => {
                    // Exactly the scenario the pre-existing CompactionStarted
                    // race-guard tests model: compaction_started/
                    // compact_boundary can arrive out of order relative to
                    // an unrelated, already-streaming turn. A manual trigger
                    // alone must NOT be read as "this pane's turn is the
                    // synthetic /compact one" — only pendingCompactTurn can
                    // say that.
                    const s0 = streaming(100); // ordinary turn, not "/compact"
                    const r = update(s0, {
                        type: "CompactionBoundary",
                        trigger: "manual",
                        preTokens: 30_000,
                        postTokens: 2_000,
                        durationMs: 5_000,
                        at: 200,
                    }, 200);
                    expect(r.state.turnPhase.kind).toBe("Streaming");
                });

                it("reports 'stopped', not 'completed', when the user hit Esc during a manual compaction", () => {
                    const s0 = compactStreaming(100);
                    const interrupting = update(s0, { type: "RequestStop", at: 150 }, 150).state;
                    const r = update(interrupting, {
                        type: "CompactionBoundary",
                        trigger: "manual",
                        preTokens: 30_000,
                        postTokens: 2_000,
                        durationMs: 5_000,
                        at: 200,
                    }, 200);
                    expect(r.state.turnPhase).toEqual({ kind: "Done", outcome: "stopped", finishedAt: 200 });
                });

                it("is a no-op on turnPhase when the pane is already idle (stale/late boundary after the turn ended some other way)", () => {
                    const r = update(mk(), {
                        type: "CompactionBoundary",
                        trigger: "manual",
                        preTokens: 30_000,
                        postTokens: 2_000,
                        durationMs: 5_000,
                        at: 100,
                    }, 100);
                    expect(r.state.turnPhase.kind).toBe("Idle");
                });
            });
        });

        describe("TokensIn heuristic suppression", () => {
            it("fires the heuristic normally with source: heuristic when no real boundary landed", () => {
                const s0 = update(mk(), { type: "TokensIn", input: 50_000 }, 100).state;
                const r = update(s0, { type: "TokensIn", input: 1_000 }, 200);
                expect(r.events).toContainEqual({
                    type: "context-compacted",
                    tokensBefore: 50_000,
                    tokensAfter: 1_000,
                    source: "heuristic",
                });
            });

            it("never compares against a reading restored from history (the false 'compacted 17m → 300k' card)", () => {
                // A seed describes a session this process didn't watch: it may
                // have been resumed, replaced or compacted since. Only readings
                // this process measured are a "before".
                const seeded = update(mk(), { type: "ReconcileContextFromHistory", tokens: 800_000 }, 100).state;
                const r = update(seeded, { type: "TokensIn", input: 60_000 }, 200);
                expect(r.events.filter((e) => e.type === "context-compacted")).toEqual([]);
                expect(r.state.context).toMatchObject({ tokens: 60_000, source: "live" });
            });

            it("after a compaction the next call is the baseline; a later drop from it is measured", () => {
                let s = update(mk(), { type: "TokensIn", input: 50_000 }, 100).state;
                s = update(s, { type: "CompactionBoundary", trigger: "auto", preTokens: 50_000, postTokens: 4_000, durationMs: 1, at: 150 }, 150).state;
                s = update(s, { type: "TokensIn", input: 40_000 }, 200).state;
                // Outside the suppression window, a later ≥50% drop is a compaction.
                const r = update(s, { type: "TokensIn", input: 1_000 }, 400_000);
                expect(r.events).toContainEqual({ type: "context-compacted", tokensBefore: 40_000, tokensAfter: 1_000, source: "heuristic" });
            });

            it("suppresses the heuristic shortly after a real CompactionBoundary landed", () => {
                const s0 = update(mk(), { type: "TokensIn", input: 50_000 }, 100).state;
                const s1 = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 150,
                }, 150).state;
                // Next turn's TokensIn shows the post-compaction fill growing
                // back up but still nowhere near the ORIGINAL 50k baseline —
                // the boundary cleared the reading, so there is no "before"
                // and this couldn't trip the ≥50% heuristic on its own, but
                // the suppression guard is the belt-and-braces check under
                // test here regardless.
                const r = update(s1, { type: "TokensIn", input: 20_000 }, 200);
                const compactionEvents = r.events.filter((e) => e.type === "context-compacted");
                expect(compactionEvents).toEqual([]);
            });

            it("re-arms the heuristic once the suppression window has elapsed", () => {
                const s0 = update(mk(), { type: "TokensIn", input: 50_000 }, 100).state;
                const s1 = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 150,
                }, 150).state;
                // Grow context back past 10k, then simulate a LATER, genuinely
                // new compaction (another ≥50% drop) well past the
                // suppression window (150 + 120_000ms).
                const s2 = update(s1, { type: "TokensIn", input: 40_000 }, 1_000).state;
                const r = update(s2, { type: "TokensIn", input: 1_000 }, 400_000);
                expect(r.events).toContainEqual({
                    type: "context-compacted",
                    tokensBefore: 40_000,
                    tokensAfter: 1_000,
                    source: "heuristic",
                });
            });
        });

        describe("compacting cleared by other lifecycle transitions (reagent P1, PR #2378)", () => {
            // If compact_boundary never arrives — the CLI crashes mid-compaction,
            // the network drops, or a reconnect/truncate race intervenes — only
            // clearing `compacting` on CompactionBoundary would strand the
            // composer strip showing "Compacting… Ns" forever, surviving
            // reconnects and every subsequent turn. Each of these four
            // transitions must clear it independently.

            it("StreamUnsubscribe clears compacting while a turn was working", () => {
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.compacting).not.toBeNull();
                const r = update(s0, { type: "StreamUnsubscribe", at: 200 }, 200);
                expect(r.state.compacting).toBeNull();
            });

            it("TurnEnd clears compacting", () => {
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "auto", at: 150 }, 150).state;
                expect(s0.compacting).not.toBeNull();
                const r = update(s0, { type: "TurnEnd", stats: null }, 200);
                expect(r.state.compacting).toBeNull();
            });

            it("TurnReset clears compacting", () => {
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.compacting).not.toBeNull();
                const r = update(s0, { type: "TurnReset" }, 200);
                expect(r.state.compacting).toBeNull();
            });

            it("RequestStop deliberately does NOT clear compacting (codex P2, round 3)", () => {
                // An earlier version of this fix cleared compacting here,
                // per a since-superseded reagent finding — but RequestStop
                // only sends a SIGINT, it doesn't confirm the turn actually
                // ended. See the StopFailed test below for why eagerly
                // clearing here was wrong.
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "auto", at: 150 }, 150).state;
                expect(s0.compacting).not.toBeNull();
                const r = update(s0, { type: "RequestStop", at: 200 }, 200);
                expect(r.state.turnPhase.kind).toBe("Interrupting");
                expect(r.state.compacting).not.toBeNull();
            });

            it("StopFailed rolling back to Streaming preserves compacting, since it was never actually interrupted", () => {
                // Codex P2 on PR #2378 (round 3): if RequestStop HAD cleared
                // compacting eagerly, this exact sequence would have lost the
                // "Compacting…" status/timer for a compaction that kept
                // running unaffected the whole time (the SIGINT never
                // landed) — plus re-enabled the stream-stuck watchdog using
                // a stale activity timestamp.
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "auto", at: 150 }, 150).state;
                const s1 = update(s0, { type: "RequestStop", at: 200 }, 200).state;
                expect(s1.turnPhase.kind).toBe("Interrupting");
                const r = update(s1, { type: "StopFailed" }, 300);
                expect(r.state.turnPhase.kind).toBe("Streaming");
                expect(r.state.compacting).toEqual({ trigger: "auto", startedAt: 150 });
            });

            it("FailureObserved clears compacting when it ends the turn (reagent P1, round 3)", () => {
                // A backend failure classification (e.g. the CLI erroring out
                // partway through) can arrive mid-compaction just like any
                // other turn-ending transition — same bug class as the four
                // above, just reached via an error instead of a clean exit.
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.compacting).not.toBeNull();
                const failure: AgentFailure = { code: "rate_limited", title: "Rate limited", detail: "429", retryable: true };
                const r = update(s0, { type: "FailureObserved", failure, at: 200 });
                expect(r.state.turnPhase).toEqual({ kind: "Done", outcome: "errored", finishedAt: 200 });
                expect(r.state.compacting).toBeNull();
            });

            it("FailureObserved does NOT clear compacting when it's a stray/late no-op (turn already ended)", () => {
                // If the turn already ended, FailureObserved leaves turnPhase
                // alone (per its own existing "stray/late event" handling) —
                // compacting should be left alone too, since nothing about
                // this transition is authoritative in that case.
                const s0 = mk(); // Idle, not working
                const failure: AgentFailure = { code: "rate_limited", title: "Rate limited", detail: "429", retryable: true };
                const r = update(s0, { type: "FailureObserved", failure, at: 200 });
                expect(r.events).toContainEqual({ type: "failure-observed", code: "rate_limited", turnWasEnded: false });
                expect(r.state.compacting).toBe(s0.compacting);
            });

            it("InterruptTimeoutElapsed clears compacting (reagent + codex P1, round 4)", () => {
                // A fifth authoritative terminal transition found across
                // three review rounds: round 3 deliberately stopped
                // RequestStop from clearing compacting (so it survives a
                // failed stop attempt) — but if the interrupt instead TIMES
                // OUT, that IS authoritative and must clear it.
                const s0 = ready(100);
                const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
                const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state;
                const s3 = update(s2, { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                const s4 = update(s3, { type: "RequestStop", at: 200 }).state;
                expect(s4.compacting).not.toBeNull();
                const deadline = 200 + INTERRUPT_TIMEOUT_MS;
                const r = update(s4, { type: "InterruptTimeoutElapsed", at: deadline });
                expect(r.state.turnPhase.kind).toBe("Done");
                expect(r.state.compacting).toBeNull();
            });

            it("SubmitTimeoutElapsed clears compacting (defensive completeness, round 4)", () => {
                const s0 = ready(100);
                const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
                expect(s1.turnPhase.kind).toBe("Submitting");
                const s2 = update(s1, { type: "CompactionStarted", trigger: "auto", at: 120 }, 120).state;
                expect(s2.compacting).not.toBeNull();
                const deadline = 110 + SUBMIT_TIMEOUT_MS;
                const r = update(s2, { type: "SubmitTimeoutElapsed", at: deadline });
                expect(r.state.turnPhase.kind).toBe("Done");
                expect(r.state.compacting).toBeNull();
            });

            it("ReconcileTurnActive(active: false) clears compacting (reagent + codex P1, round 4)", () => {
                // The backend has authoritatively confirmed no turn is
                // active — whatever compaction the frontend still thought
                // was in flight is stale.
                const s0 = update(streaming(100), { type: "CompactionStarted", trigger: "manual", at: 150 }, 150).state;
                expect(s0.compacting).not.toBeNull();
                const r = update(s0, { type: "ReconcileTurnActive", at: 200, active: false });
                expect(r.state.turnPhase.kind).toBe("Idle");
                expect(r.state.compacting).toBeNull();
            });

            it("TurnStartFailed clears compacting (codex P2, round 8)", () => {
                // A compaction_started ping can land while a NEW turn
                // attempt is briefly Submitting (round 5's workingFromPhase
                // gate explicitly permits Submitting) -- if that turn's own
                // initiating RPC then fails synchronously, this arm must
                // not leave the stale compacting state behind.
                const s0 = ready(100);
                const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
                expect(s1.turnPhase.kind).toBe("Submitting");
                const s2 = update(s1, { type: "CompactionStarted", trigger: "manual", at: 120 }, 120).state;
                expect(s2.compacting).not.toBeNull();
                const r = update(s2, { type: "TurnStartFailed" });
                expect(r.state.turnPhase.kind).toBe("Idle");
                expect(r.state.compacting).toBeNull();
            });
        });

        describe("CompactionBoundary preserves a newer compaction against an out-of-order delayed boundary (codex P2, round 8)", () => {
            // compact_boundary (NDJSON) and compaction_started (MPS) travel
            // over two independent transports with no ordering guarantee.
            // If compaction N+1 has already started before a DELAYED
            // boundary for compaction N arrives, clearing `compacting`
            // unconditionally would wipe the genuinely active N+1 state
            // using stale N data.

            it("does NOT clear compacting when the boundary's frameTimestamp predates the current compaction's start", () => {
                // Compaction N+1 started at t=1000 (frameTimestamp-space).
                const s0 = update(streaming(100), {
                    type: "CompactionStarted",
                    trigger: "manual",
                    at: 1_000,
                }, 1_000).state;
                expect(s0.compacting).toEqual({ trigger: "manual", startedAt: 1_000 });
                // A delayed boundary for the OLDER compaction N arrives,
                // whose own completion (frameTimestamp) was BEFORE N+1 started.
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 1_500,
                    frameTimestamp: new Date(500).toISOString(),
                }, 1_500);
                expect(r.state.compacting).toEqual({ trigger: "manual", startedAt: 1_000 });
                // lastCompactionBoundaryAt takes the boundary's own parsed
                // completion time (frameTimestamp: 500), not the frontend
                // receipt time (at: 1_500) -- see codex round 10.
                expect(r.state.lastCompactionBoundaryAt).toBe(500);
                // The context reading is NOT overwritten with this older
                // boundary's postTokens -- the newer N+1 compaction is
                // confirmed still active, so showing its stale 3_000
                // reading would be a regression from whatever context-fill
                // value was live before it (reagent P2, round 11).
                expect(r.state.context).toBe(null);
            });

            it("preserves the pre-existing reading (not just null) against a stale delayed boundary (reagent P2, round 11)", () => {
                // A real, live context-fill value (8_000) is already
                // established before the delayed older boundary shows up,
                // so this proves the fix preserves whatever was actually
                // live -- not merely that it happens to stay null.
                const withTokens = update(streaming(100), { type: "TokensIn", input: 8_000 }, 500).state;
                const s0 = update(withTokens, {
                    type: "CompactionStarted",
                    trigger: "manual",
                    at: 1_000,
                }, 1_000).state;
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 1_500,
                    frameTimestamp: new Date(500).toISOString(),
                }, 1_500);
                expect(r.state.compacting).toEqual({ trigger: "manual", startedAt: 1_000 });
                expect(r.state.context?.tokens).toBe(8_000);
            });

            it("DOES clear compacting when the boundary's frameTimestamp is at or after the current compaction's start", () => {
                const s0 = update(streaming(100), {
                    type: "CompactionStarted",
                    trigger: "manual",
                    at: 1_000,
                }, 1_000).state;
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 1_500,
                    frameTimestamp: new Date(1_200).toISOString(),
                }, 1_500);
                expect(r.state.compacting).toBeNull();
            });

            it("falls back to clearing when frameTimestamp is absent (can't tell, avoid a permanent stuck state)", () => {
                const s0 = update(streaming(100), {
                    type: "CompactionStarted",
                    trigger: "manual",
                    at: 1_000,
                }, 1_000).state;
                const r = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 1_500,
                    frameTimestamp: null,
                }, 1_500);
                expect(r.state.compacting).toBeNull();
            });

            it("records the boundary's own parsed completion time, not the frontend's receipt time, as lastCompactionBoundaryAt (codex P2, round 10)", () => {
                // Compaction N truly completed at t=50 (frameTimestamp), but
                // its boundary frame is delayed in delivery and only
                // reaches the frontend at t=500 (receipt/`at`). Compaction
                // N+1's own `CompactionStarted.at` carries the MPS payload's
                // embedded TRUE start time (t=60), not a receipt timestamp
                // -- comparing it against N's inflated receipt-time
                // boundary (500) instead of N's true completion (50) would
                // falsely reject N+1's genuinely-later start.
                const s0 = update(streaming(30), {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 50_000,
                    postTokens: 3_000,
                    durationMs: 9_000,
                    at: 500,
                    frameTimestamp: new Date(50).toISOString(),
                }, 500).state;
                expect(s0.lastCompactionBoundaryAt).toBe(50);
                const r = update(s0, { type: "CompactionStarted", trigger: "auto", at: 60 }, 600);
                expect(r.state.compacting).toEqual({ trigger: "auto", startedAt: 60 });
            });
        });

        describe("StreamWatchdogTick is suspended while compacting (codex P1, PR #2378 round 2)", () => {
            // CompactionStarted only bumps lastEventMs ONCE, at the start —
            // it is never re-bumped on later ticks. The captured real
            // example (spec doc §2) took ~232s, comfortably past both
            // STUCK_THRESHOLD_MS (45s) and LIVENESS_RECOVERY_MS (180s).
            // Without an explicit suspension, a perfectly normal compaction
            // would trip a false "stream-stuck" diagnostic and then get
            // force-demoted from Streaming to Idle out from under an
            // actively-compacting turn.

            it("emits no stream-stuck diagnostic past STUCK_THRESHOLD_MS while compacting", () => {
                const s0 = update(streaming(1_000), { type: "CompactionStarted", trigger: "auto", at: 1_000 }, 1_000).state;
                const tick = 1_000 + STUCK_THRESHOLD_MS + 1_000;
                const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
                expect(r.state).toBe(s0);
                expect(r.events).toEqual([]);
            });

            it("does NOT force-recover Streaming -> Idle past LIVENESS_RECOVERY_MS while compacting", () => {
                const s0 = update(streaming(1_000), { type: "CompactionStarted", trigger: "manual", at: 1_000 }, 1_000).state;
                const tick = 1_000 + LIVENESS_RECOVERY_MS + 60_000; // well past, e.g. a ~232s-class compaction
                const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
                expect(r.state.turnPhase.kind).toBe("Streaming");
                expect(r.state).toBe(s0);
                expect(r.events).toEqual([]);
            });

            it("re-arms the watchdog once CompactionBoundary clears compacting", () => {
                const s0 = update(streaming(1_000), { type: "CompactionStarted", trigger: "manual", at: 1_000 }, 1_000).state;
                const s1 = update(s0, {
                    type: "CompactionBoundary",
                    trigger: "manual",
                    preTokens: 100_000,
                    postTokens: 5_000,
                    durationMs: 232_000,
                    at: 233_000,
                }, 233_000).state;
                expect(s1.compacting).toBeNull();
                // lastEventMs was bumped to 233_000 by the boundary itself
                // (see the CompactionBoundary reducer case) — a tick past
                // LIVENESS_RECOVERY_MS measured from THAT point behaves
                // exactly like the ordinary hang-recovery case.
                const tick = 233_000 + LIVENESS_RECOVERY_MS + 1_000;
                const r = update(s1, { type: "StreamWatchdogTick", nowMs: tick });
                expect(r.state.turnPhase.kind).toBe("Idle");
                expect(r.events[0]).toMatchObject({ type: "working-recovered" });
            });
        });
    });

});
