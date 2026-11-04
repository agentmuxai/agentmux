// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Small pure steps the command handlers share. */

import { AgentPaneState, NO_RATE_LIMIT_WAIT, type TurnCarry, type ActivityState, type ModelActivityPhase } from "./types";
import { type SessionStats, type TurnTokens } from "@/app/agent-model/types";
import { type ContextReading } from "./context-reading";
import { turnOutputTokens } from "./turn-contribution";
import { toolActivity } from "@/app/store/agent-pane-state/tool-labels";
import { planFromTodoWrite } from "@/app/store/agent-pane-state/plan";

/** A live reading taken after `t` (unix ms): it already measures what an event
 *  at `t` left behind. */
export function readingPostdates(reading: ContextReading | null, t: number): boolean {
    return reading != null && reading.source === "live" && reading.at != null && reading.at > t;
}


/** Add a finished pass's figures to its turn's carry (a new turn starts a new one). */
export function carryPass(carry: TurnCarry | null, turnId: number | undefined, stats: SessionStats | null): TurnCarry | null {
    if (turnId == null || !stats) return carry;
    const base = carry?.turnId === turnId ? carry : { turnId, passes: 0, outputTokens: 0, costUsd: 0, steps: 0 };
    return {
        turnId,
        passes: base.passes + 1,
        outputTokens: base.outputTokens + (stats.output_tokens ?? 0),
        costUsd: base.costUsd + (stats.cost_usd ?? 0),
        steps: base.steps + (stats.num_turns ?? 0),
    };
}

/** How many of a pane's recent waits for the model it keeps. */
const WAIT_SAMPLES = 20;

/** A request was waiting and the model's answer begins: keep how long it took. */
export function withWaitRecorded(a: ActivityState, nowMs: number): ActivityState {
    if (a.phase !== "requesting") return a;
    return { ...a, waits: [...(a.waits ?? []), nowMs - a.phaseSince].slice(-WAIT_SAMPLES) };
}

/** The model's phase moves on; `phaseSince` changes only with it. */
export function withPhase(a: ActivityState, phase: ModelActivityPhase, nowMs: number): ActivityState {
    return a.phase === phase ? a : { ...a, phase, phaseSince: nowMs };
}

/**
 * A tool call starts: it joins the running list; a todo list sets the plan.
 * One call is reported more than once with the same id (the CLI's streaming
 * placeholder with empty input, then the full call; ACP's updates): that is
 * an update of the entry already there, keeping its start, parent, progress
 * and output time, never a second call (#4510).
 */
export function activityWithTool(
    a: ActivityState,
    c: { name: string; id?: string; params?: Record<string, unknown>; parentId?: string },
    nowMs: number,
): ActivityState {
    const activity = toolActivity(c.name, c.params);
    const command = typeof c.params?.command === "string" && c.params.command ? { command: c.params.command } : {};
    const i = c.id != null ? a.tools.findIndex((t) => t.id === c.id) : -1;
    const tools =
        i >= 0
            ? a.tools.map((t, j) => (j === i ? { ...t, activity, name: c.name, ...command, ...(c.parentId ? { parentId: c.parentId } : {}) } : t))
            : [...a.tools, { id: c.id ?? null, activity, name: c.name, ...command, startedAt: nowMs, ...(c.parentId ? { parentId: c.parentId } : {}) }];
    // Only a call that carries its list sets the plan: the empty placeholder
    // must not wipe the plan already there.
    if (c.name === "TodoWrite" && Array.isArray(c.params?.todos)) {
        return { ...a, tools, plan: planFromTodoWrite(c.params), planAt: nowMs };
    }
    return { ...a, tools };
}

/** A tool call ends: by id when known, else the oldest. */
export function activityWithoutTool(a: ActivityState, id: string | undefined): ActivityState {
    if (a.tools.length === 0) return a;
    const i = id != null ? a.tools.findIndex((t) => t.id === id) : 0;
    if (i < 0) return a;
    return { ...a, tools: a.tools.filter((_, j) => j !== i) };
}

/** The pass is over: nothing runs; the plan stays for the turn's next pass. */
export function settledActivity(a: ActivityState): ActivityState {
    return a.phase == null && a.tools.length === 0 ? a : { ...a, phase: null, phaseSince: 0, tools: [] };
}

/**
 * The turn and pass a call's tokens belong to, from the pane's ledger, so the
 * live counter never adds a pass srv has already counted (turn-ledger.ts
 * `turnLiveOutput`). srv reports a pass before it forwards any of its lines,
 * so a call always belongs to the ledger's latest pass: the running one, or,
 * for a line that arrives late, the one that just ended.
 */
export function ledgerStamp(state: AgentPaneState): Pick<TurnTokens, "ledgerTurnId" | "ledgerPass"> {
    const l = state.turnLedger;
    return l ? { ledgerTurnId: l.turnId, ledgerPass: l.passes } : {};
}

/**
 * Bump `lastEventMs` to "now" only while the stream is subscribed. Used
 * by tool/token transitions which are observable stream activity. Skip
 * the bump when the stream is unsubscribed — those would correspond to
 * stale commands and shouldn't reset the watchdog clock.
 *
 * PR G: the subscribed gate is now `state.lastEventMs !== null` (was
 * `state.streaming.active`; they always moved in lockstep). If the
 * phase is Streaming, also update its `lastEventMs` and apply
 * `toolsDelta` to `toolsActive` (clamped ≥ 0).
 */
export function bumpEvent(
    state: AgentPaneState,
    nowMs: number,
    toolsDelta: number,
): AgentPaneState {
    if (state.lastEventMs == null) return state;
    const next: AgentPaneState = { ...state, lastEventMs: nowMs };
    if (next.turnPhase.kind === "Streaming") {
        next.turnPhase = {
            ...next.turnPhase,
            lastEventMs: nowMs,
            toolsActive: Math.max(0, next.turnPhase.toolsActive + toolsDelta),
            ...NO_RATE_LIMIT_WAIT,
        };
    } else if (
        next.turnPhase.kind === "Submitting" ||
        next.turnPhase.kind === "Idle" ||
        next.turnPhase.kind === "Disconnected" ||
        (next.turnPhase.kind === "Done" && next.turnPhase.outcome === "completed")
    ) {
        // codex P1 on #987: `StreamSubscribe` fires once at mount, so
        // subsequent turns enter Submitting and then no transition
        // arm ever advances them to Streaming — chunks/tokens/tools
        // arrive but phase stayed stuck. First stream activity from
        // Submitting is the actual "stream is producing" signal.
        //
        // Also promote from Idle / Disconnected: live tool/token activity
        // (this helper only runs for live stream events, never history) is
        // proof the agent is working. After a stream drop + resubscribe
        // (agent kill+respawn during a long upstream stall) the phase lands
        // in Idle/Disconnected; without this the working indicator stays off
        // while output streams.
        // Done.completed is included: session_end fires after every model API
        // round, so Done.completed can mean "first round of a multi-round
        // tool-continuation finished" — re-enter Streaming on next activity.
        // Done.errored/stopped/interrupted excluded: a late stray event must
        // not revive the indicator on a failed/aborted turn.
        // Interrupting is intentionally excluded (user is stopping).
        next.turnPhase = {
            kind: "Streaming",
            bufferSize: next.streaming.bufferSize,
            toolsActive: Math.max(0, toolsDelta),
            lastEventMs: nowMs,
        };
    }
    return next;
}

/**
 * Merge the turn's token totals into sessionStats. Prefer the result
 * event's whole-turn totals; fall back to the live turn-tokens only when
 * the result reported no/zero usage (hence `||`, not `??`). The live
 * input is the last call's, the live output the turn's total
 * (turnOutputTokens). Codex emits usage only on the stats branch (no
 * live tokens), so it's unaffected.
 */
export function mergeStats(
    stats: AgentPaneState["sessionStats"],
    tokens: AgentPaneState["turnTokens"],
): AgentPaneState["sessionStats"] {
    // The turn's output over all its calls, as the working row counted it.
    const liveOutput = turnOutputTokens(tokens);
    if (stats) {
        return {
            ...stats,
            input_tokens: stats.input_tokens || tokens?.input,
            output_tokens: stats.output_tokens || liveOutput,
            // Same fallback logic as input_tokens/output_tokens above,
            // applied to the cache breakdown — prefer the result event's
            // own breakdown, fall back to the live turn-tokens' breakdown.
            fresh_input_tokens: stats.fresh_input_tokens ?? tokens?.freshInput,
            cache_creation_input_tokens: stats.cache_creation_input_tokens ?? tokens?.cacheCreation,
            cache_read_input_tokens: stats.cache_read_input_tokens ?? tokens?.cacheRead,
        };
    }
    if (tokens) {
        return {
            input_tokens: tokens.input,
            output_tokens: liveOutput,
            fresh_input_tokens: tokens.freshInput,
            cache_creation_input_tokens: tokens.cacheCreation,
            cache_read_input_tokens: tokens.cacheRead,
        };
    }
    return null;
}

/**
 * Sum a just-completed turn's merged stats into the pane's running total.
 * Unlike mergeStats (which replaces sessionStats per turn), this adds —
 * missing numeric fields are treated as 0, num_turns falls back to 1 per
 * completed turn so a CLI that omits it still counts as one query.
 * See SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md.
 */
export function accumulateStats(
    totals: AgentPaneState["sessionTotals"],
    merged: AgentPaneState["sessionStats"],
): AgentPaneState["sessionTotals"] {
    if (!merged) return totals;
    return {
        cost_usd: (totals?.cost_usd ?? 0) + (merged.cost_usd ?? 0),
        duration_ms: (totals?.duration_ms ?? 0) + (merged.duration_ms ?? 0),
        input_tokens: (totals?.input_tokens ?? 0) + (merged.input_tokens ?? 0),
        output_tokens: (totals?.output_tokens ?? 0) + (merged.output_tokens ?? 0),
        // Same accumulate-not-replace treatment as input_tokens/output_tokens
        // above, for the cache breakdown. See fresh_input_tokens' doc comment
        // in agent-model/stream-events.ts for why this split matters.
        fresh_input_tokens: (totals?.fresh_input_tokens ?? 0) + (merged.fresh_input_tokens ?? 0),
        cache_creation_input_tokens:
            (totals?.cache_creation_input_tokens ?? 0) + (merged.cache_creation_input_tokens ?? 0),
        cache_read_input_tokens: (totals?.cache_read_input_tokens ?? 0) + (merged.cache_read_input_tokens ?? 0),
        num_turns: (totals?.num_turns ?? 0) + (merged.num_turns ?? 1),
    };
}
