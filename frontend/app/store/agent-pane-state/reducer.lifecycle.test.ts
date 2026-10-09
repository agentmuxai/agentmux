// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane reducer: stream and turn lifecycle, tools.

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { isWorking } from "./types";
import { streaming, mk, ready } from "./reducer-test-fixtures";

describe("agent-pane-state reducer", () => {
    describe("Stream lifecycle", () => {
        it("StreamSubscribe sets lastEventMs + streaming telemetry", () => {
            const r = update(mk(), { type: "StreamSubscribe", at: 100 });
            // PR G: subscribed-ness is `lastEventMs !== null`; the
            // legacy `streaming.active` boolean was dropped.
            expect(r.state.lastEventMs).toBe(100);
            expect(r.state.streaming.lastEventTime).toBe(100);
            expect(r.events[0]).toMatchObject({ type: "stream-subscribed", at: 100 });
        });

        it("StreamUnsubscribe clears subscription and forces working turn into Disconnected", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(isWorking(s1)).toBe(true);
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            expect(r.state.lastEventMs).toBe(null);
            expect(isWorking(r.state)).toBe(false);
            expect(r.state.turnPhase.kind).toBe("Disconnected");
        });

        it("StreamFlushObserved bumps bufferSize when subscribed", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s0, { type: "StreamFlushObserved", addedCount: 3, at: 110 });
            expect(r.state.streaming.bufferSize).toBe(3);
            expect(r.state.streaming.lastEventTime).toBe(110);
        });

        it("StreamFlushObserved is no-op when stream unsubscribed", () => {
            const start = mk();
            const r = update(start, { type: "StreamFlushObserved", addedCount: 3, at: 110 });
            // Reducer must return SAME reference when no work was done.
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });
    });

    describe("ReconcileTurnActive (mount-time reconciliation)", () => {
        it("promotes a fresh Idle pane to Streaming when the backend reports a turn in flight", () => {
            const start = mk();
            expect(start.turnPhase.kind).toBe("Idle");
            const r = update(start, { type: "ReconcileTurnActive", at: 100, active: true });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            expect(r.events[0]).toMatchObject({ type: "turn-active-reconciled" });
        });

        it("active: false is a no-op — Idle is already correct", () => {
            const start = mk();
            const r = update(start, { type: "ReconcileTurnActive", at: 100, active: false });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("does not override a phase a real event already produced", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const r = update(s1, { type: "ReconcileTurnActive", at: 120, active: true });
            expect(r.state).toBe(s1);
            expect(r.events).toEqual([]);
        });

        // reagent P1 on the PR that added the focus-triggered reconcile
        // (SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md):
        // without this, a pane showing "Worked" while backgrounded, whose
        // genuinely-new turn's live start signal was ALSO missed, would
        // silently no-op forever on this authoritative RPC response.
        it("promotes a settled Done.completed episode to Streaming — the missed-live-turn-start case", () => {
            const s0 = streaming(100);
            const s1 = update(s0, { type: "TurnEnd", stats: null }).state;
            expect(s1.turnPhase).toMatchObject({ kind: "Done", outcome: "completed" });
            const r = update(s1, { type: "ReconcileTurnActive", at: 200, active: true });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            expect(r.events[0]).toMatchObject({ type: "turn-active-reconciled" });
        });

        it("does NOT promote Done.stopped/errored — same standard as StreamFlushObserved", () => {
            const s0 = streaming(100);
            // Interrupting -> TurnEnd yields Done.stopped.
            const interrupting = update(s0, { type: "RequestStop", at: 150 }).state;
            expect(interrupting.turnPhase.kind).toBe("Interrupting");
            const stopped = update(interrupting, { type: "TurnEnd", stats: null }).state;
            expect(stopped.turnPhase).toMatchObject({ kind: "Done", outcome: "stopped" });
            const r = update(stopped, { type: "ReconcileTurnActive", at: 200, active: true });
            expect(r.state).toBe(stopped);
            expect(r.events).toEqual([]);
        });

        it("does not require the stream to be subscribed yet (unlike TurnStart)", () => {
            const start = mk();
            expect(start.lastEventMs).toBe(null);
            const r = update(start, { type: "ReconcileTurnActive", at: 100, active: true });
            expect(r.state.turnPhase.kind).toBe("Streaming");
        });

        // active: false — downward reconciliation (completes #2005's symmetry).
        // See docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md.
        it("demotes a stuck Streaming phase to Idle when the backend reports the turn ended", () => {
            const s = streaming(100);
            expect(s.turnPhase.kind).toBe("Streaming");
            const r = update(s, { type: "ReconcileTurnActive", at: 200, active: false });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.events[0]).toMatchObject({ type: "turn-inactive-reconciled", at: 200 });
        });

        it("clears currentTool when demoting a stuck Streaming turn, and keeps its tokens for the session_end that follows", () => {
            let s = streaming(100);
            s = update(s, { type: "ToolStart", name: "Bash", arg: "ls" }).state;
            s = update(s, { type: "TokensIn", input: 500, model: "claude-sonnet-5" }).state;
            expect(s.currentTool).not.toBe(null);
            const r = update(s, { type: "ReconcileTurnActive", at: 200, active: false });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state.currentTool).toBe(null);
            expect(r.state.currentToolArg).toBe(null);
            expect(r.state.turnTokens).toMatchObject({ input: 500 });
        });

        it("demotes Streaming even with a tool active — backend turn_active=false is authoritative (unlike the timeout watchdog)", () => {
            let s = streaming(100);
            s = update(s, { type: "ToolStart", name: "Bash", arg: "sleep 999" }).state;
            // The liveness watchdog would REFUSE to recover a tool-active turn
            // (a long tool legitimately keeps it alive); a backend result event
            // is ground truth, so this demotes regardless.
            const r = update(s, { type: "ReconcileTurnActive", at: 200, active: false });
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("active: false leaves Submitting untouched — that's SUBMIT_TIMEOUT's job (and where the send-race lives)", () => {
            const s = update(ready(100), { type: "TurnStart", at: 110 }).state;
            expect(s.turnPhase.kind).toBe("Submitting");
            const r = update(s, { type: "ReconcileTurnActive", at: 200, active: false });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });

        it("active: false leaves Done untouched (already terminal)", () => {
            const s = update(streaming(100), { type: "TurnEnd", stats: null }).state;
            expect(s.turnPhase.kind).toBe("Done");
            const r = update(s, { type: "ReconcileTurnActive", at: 200, active: false });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });

        it("active: true still promotes from Idle after a prior demote (round-trip)", () => {
            const demoted = update(streaming(100), { type: "ReconcileTurnActive", at: 200, active: false }).state;
            expect(demoted.turnPhase.kind).toBe("Idle");
            const r = update(demoted, { type: "ReconcileTurnActive", at: 300, active: true });
            expect(r.state.turnPhase.kind).toBe("Streaming");
        });
    });

    describe("ReconcileContextFromHistory (mount-time reconciliation)", () => {
        it("seeds the context reading from a fresh (never-set) pane", () => {
            const start = mk();
            expect(start.context).toBe(null);
            const r = update(start, { type: "ReconcileContextFromHistory", tokens: 4200 });
            expect(r.state.context).toMatchObject({ tokens: 4200, source: "history" });
            expect(r.events[0]).toMatchObject({ type: "context-reconciled-at-mount", tokens: 4200 });
        });

        it("resolves the window for the history's model at mount — 1M for Sonnet 5.5, not 200K", () => {
            // The owner's report: a freshly opened Sonnet 5.5 pane read "/ 200k"
            // because nothing resolved a window before the first live call.
            const r = update(mk(), {
                type: "ReconcileContextFromHistory",
                tokens: 300_000,
                model: "claude-sonnet-5-5",
                at: 1_700_000_000_000,
            });
            expect(r.state.context).toEqual({
                tokens: 300_000,
                model: "claude-sonnet-5-5",
                window: 1_000_000,
                windowSource: "model",
                source: "history",
                at: 1_700_000_000_000,
                switchedTo: null,
            });
            expect(r.state.lastContextModel).toBe("claude-sonnet-5-5");
        });

        it("prefers a window the history's result frames reported over the model table", () => {
            const r = update(mk(), {
                type: "ReconcileContextFromHistory",
                tokens: 300_000,
                model: "claude-sonnet-4-6",
                reportedWindows: { "claude-sonnet-4-6": 1_000_000 },
            });
            expect(r.state.context).toMatchObject({ window: 1_000_000, windowSource: "reported" });
            expect(r.state.reportedContextWindows).toEqual({ "claude-sonnet-4-6": 1_000_000 });
        });

        it("refuses a seed larger than its window (a turn total, the 17m bug) and says why", () => {
            const r = update(mk(), {
                type: "ReconcileContextFromHistory",
                tokens: 17_000_000,
                model: "claude-sonnet-5-5",
            });
            expect(r.state.context).toBeNull();
            expect(r.events).toEqual([
                {
                    type: "context-reading-rejected",
                    tokens: 17_000_000,
                    window: 1_000_000,
                    model: "claude-sonnet-5-5",
                    source: "history",
                    reason: "tokens exceed the window",
                },
            ]);
        });

        it("refuses a seed above every known window when the model is unknown", () => {
            const r = update(mk(), { type: "ReconcileContextFromHistory", tokens: 17_000_000 });
            expect(r.state.context).toBeNull();
            expect(r.events[0]).toMatchObject({ type: "context-reading-rejected", reason: "tokens exceed every known window" });
        });

        it("does not override a value a live TokensIn already set", () => {
            const s0 = update(mk(), { type: "TokensIn", input: 900, model: "claude-sonnet-5" }).state;
            expect(s0.context?.tokens).toBe(900);
            const r = update(s0, { type: "ReconcileContextFromHistory", tokens: 4200 });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("still merges the history's reported windows under a live reading, live entries winning", () => {
            let s = update(mk(), { type: "ContextWindowsReported", windows: { "claude-opus-5-5": 1_000_000 } }).state;
            s = update(s, { type: "TokensIn", input: 900, model: "claude-sonnet-4-6" }).state;
            expect(s.context).toMatchObject({ window: 200_000, windowSource: "model" });
            const r = update(s, {
                type: "ReconcileContextFromHistory",
                tokens: 4200,
                reportedWindows: { "claude-sonnet-4-6": 1_000_000, "claude-opus-5-5": 500_000 },
            });
            expect(r.state.context).toMatchObject({ tokens: 900, window: 1_000_000, windowSource: "reported" });
            expect(r.state.reportedContextWindows).toEqual({
                "claude-sonnet-4-6": 1_000_000,
                "claude-opus-5-5": 1_000_000,
            });
        });

        it("does not override an earlier reconciliation either (first-wins)", () => {
            const s0 = update(mk(), { type: "ReconcileContextFromHistory", tokens: 4200 }).state;
            const r = update(s0, { type: "ReconcileContextFromHistory", tokens: 999 });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });
    });

    describe("Turn lifecycle invariants", () => {
        it("TurnStart while stream unsubscribed is suppressed", () => {
            const start = mk();
            const r = update(start, { type: "TurnStart", at: 100 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "turn-start-suppressed" });
        });

        it("TurnStart while subscribed transitions to Submitting + clears stale stats", () => {
            const s0 = ready(100);
            const sWithStats = { ...s0, sessionStats: { input_tokens: 50, output_tokens: 100 } };
            const r = update(sWithStats, { type: "TurnStart", at: 110 });
            expect(r.state.turnPhase.kind).toBe("Submitting");
            expect(isWorking(r.state)).toBe(true);
            expect(r.state.sessionStats).toBe(null);
        });

        it("TurnEnd clears tool/tokens and lands in Done in one shot", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "ToolStart", name: "Read" }).state;
            const s3 = update(s2, { type: "TokensIn", input: 50 }).state;
            const s4 = update(s3, { type: "TokensOut", output: 200 }).state;
            const s5 = update(s4, { type: "RequestStop", at: 120 }).state;
            const r = update(s5, {
                type: "TurnEnd",
                stats: null,
            });
            expect(isWorking(r.state)).toBe(false);
            expect(r.state.turnPhase.kind).toBe("Done");
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
            // Stats merged from live tokens (mergeStats fallback path).
            expect(r.state.sessionStats).toEqual({ input_tokens: 50, output_tokens: 200 });
            expect(r.events[0]).toMatchObject({
                type: "turn-ended",
                // outcome is "stopped" because RequestStop put the phase
                // into Interrupting before TurnEnd ran. Sound subsystem
                // (SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §3.2) reads
                // outcome directly from the event without snapshotting.
                outcome: "stopped",
                statsMerged: true,
                // stoppingCleared still carries the audit signal — true
                // iff the turn ended while in Interrupting (PR G:
                // derived from `turnPhase.kind === "Interrupting"`).
                stoppingCleared: true,
            });
        });

        it("TurnEnd with explicit stats merges live token totals", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TokensIn", input: 80 }).state;
            const r = update(s2, {
                type: "TurnEnd",
                stats: { input_tokens: 0, output_tokens: 0, total_cost_usd: 0.05 } as any,
            });
            expect(r.state.sessionStats).toMatchObject({
                input_tokens: 80,
                output_tokens: 0,
                total_cost_usd: 0.05,
            });
        });

        it("TurnEnd prefers token-bearing result totals over live last-message tokens", () => {
            // Live turnTokens hold only the last message_start/message_delta
            // (TokensIn/TokensOut overwrite); the result carries the
            // cache-inclusive turn total, which must win.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TokensIn", input: 2 }).state;
            const s3 = update(s2, { type: "TokensOut", output: 300 }).state;
            const r = update(s3, {
                type: "TurnEnd",
                stats: { input_tokens: 70000, output_tokens: 512 } as any,
            });
            expect(r.state.sessionStats).toMatchObject({ input_tokens: 70000, output_tokens: 512 });
        });

        // SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md — sessionTotals must
        // accumulate across turns while sessionStats (per-turn) resets.
        it("TurnEnd accumulates sessionTotals across multiple turns, unlike per-turn sessionStats", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, {
                type: "TurnEnd",
                stats: { input_tokens: 100, output_tokens: 50, cost_usd: 0.01 } as any,
            }).state;
            expect(s2.sessionStats).toMatchObject({ input_tokens: 100, output_tokens: 50, cost_usd: 0.01 });
            expect(s2.sessionTotals).toMatchObject({
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                num_turns: 1,
            });

            // Second query in the same pane — per-turn stats reset on
            // TurnStart and are replaced (not summed) on TurnEnd, but
            // sessionTotals must add on top of the first turn's totals.
            const s3 = update(s2, { type: "TurnStart", at: 200 }).state;
            expect(s3.sessionStats).toBe(null);
            expect(s3.sessionTotals).toMatchObject({ input_tokens: 100, output_tokens: 50, cost_usd: 0.01 });

            const s4 = update(s3, {
                type: "TurnEnd",
                stats: { input_tokens: 30, output_tokens: 20, cost_usd: 0.002 } as any,
            }).state;
            // Per-turn: only reflects this second query.
            expect(s4.sessionStats).toMatchObject({ input_tokens: 30, output_tokens: 20, cost_usd: 0.002 });
            // Running total: sum of both queries.
            expect(s4.sessionTotals).toMatchObject({
                input_tokens: 130,
                output_tokens: 70,
                cost_usd: expect.closeTo(0.012, 10),
                num_turns: 2,
            });
        });

        it("TurnReset clears turn-scoped state but keeps subscription + pending", () => {
            const s0 = ready(100);
            const s1 = update(s0, {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "p1",
                text: "hello",
                at: 105,
            }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "ToolStart", name: "Edit" }).state;
            const r = update(s3, { type: "TurnReset" });
            // PR G: subscription gate is `lastEventMs !== null` (was
            // `streaming.active`). Preserved across TurnReset.
            expect(r.state.lastEventMs).not.toBeNull();
            expect(r.state.pending).toHaveLength(1); // preserved
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("TurnReset clears accumulated sessionTotals (session wipe)", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, {
                type: "TurnEnd",
                stats: { input_tokens: 100, output_tokens: 50, cost_usd: 0.01 } as any,
            }).state;
            expect(s2.sessionTotals).not.toBeNull();
            const r = update(s2, { type: "TurnReset" });
            expect(r.state.sessionStats).toBe(null);
            expect(r.state.sessionTotals).toBe(null);
        });

        it("TurnStartFailed reverts turnPhase to Idle WITHOUT wiping accumulated sessionTotals — unlike TurnReset", () => {
            // A transient send failure (no controller registered, spawn gate
            // blocked, network rejection) on an agent that already has prior
            // completed turns must not wipe the session's accumulated
            // cost/token display — reagent/codex P2 on PR #2318.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, {
                type: "TurnEnd",
                stats: { input_tokens: 100, output_tokens: 50, cost_usd: 0.01 } as any,
            }).state;
            expect(s2.sessionTotals).not.toBeNull();

            // A new, unrelated turn optimistically starts, then its own send fails.
            const s3 = update(s2, { type: "TurnStart", at: 200 }).state;
            const r = update(s3, { type: "TurnStartFailed" });

            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state.sessionTotals).toMatchObject({ input_tokens: 100, output_tokens: 50, cost_usd: 0.01 });
            expect(r.state.lastEventMs).not.toBeNull();
            expect(r.events).toEqual([{ type: "turn-start-failed" }]);
        });
    });

    describe("Tool", () => {
        it("ToolStart sets currentTool", () => {
            const r = update(mk(), { type: "ToolStart", name: "Bash" });
            expect(r.state.currentTool).toBe("Bash");
        });

        it("ToolEnd clears", () => {
            const s0 = update(mk(), { type: "ToolStart", name: "Bash" }).state;
            const r = update(s0, { type: "ToolEnd" });
            expect(r.state.currentTool).toBe(null);
        });
    });

});
