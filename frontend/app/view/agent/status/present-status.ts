// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The working row's live status: what to say, and when. A pure function of
 * what the pane knows now and what it showed last, so every rule here is a
 * table test with a fake clock.
 *
 * Ranks, first eligible wins (spec §6.4):
 *   0 needs you   a question or an approval is waiting on the user
 *   1 held        reconnecting, compacting, stopping, rate limited, a launch phase
 *   2 anomaly     a request that has waited unusually long for the model
 *   3 lead-in     the first moments of a turn something else started
 *   4 now         what is running, once it has lasted long enough to matter
 *   5 plan        the step of the agent's own todo list it is on
 *   6 goal        the session's ambient summary
 *   7 phrase      "Working…"
 *
 * Timing (the "right time"): an activity is promoted to rank 4 only after a
 * threshold, so bursts of quick calls never reach the screen; a shown line
 * stays at least DWELL before a same-or-lower rank replaces it (0–2 preempt);
 * a rank-4 line lingers HOLD after its activity ends, so the row doesn't
 * flash back to the goal between two calls.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.
 */

import type { ActivityState } from "@/app/store/agent-pane-state/types";
import { formatElapsedCompact } from "@/util/format-time";
import { planLine } from "@/app/store/agent-pane-state/plan";
import { foldActivities } from "@/app/store/agent-pane-state/tool-labels";

export const RANK = { needsYou: 0, held: 1, anomaly: 2, leadIn: 3, now: 4, plan: 5, goal: 6, phrase: 7 } as const;

export const TIMING = {
    /** A tool call shows once it has run this long. */
    toolPromoteMs: 1_500,
    /** The model writing a tool call's input. */
    composingPromoteMs: 2_000,
    /** The model writing its reply. */
    writingPromoteMs: 3_000,
    thinkingPromoteMs: 4_000,
    /** A line stays at least this long before a same-or-lower rank replaces it. */
    dwellMs: 1_200,
    /** A rank-4 line lingers this long after its activity ended. */
    holdMs: 2_000,
    /** A request that has waited this long for the model is worth saying. */
    slowRequestMs: 20_000,
    /** A single running call shows its elapsed time from here on. */
    showElapsedAfterMs: 10_000,
    /** A command that wrote output and then fell silent this long is worth saying. */
    quietToolMs: 60_000,
} as const;

export interface StatusInput {
    nowMs: number;
    /** A question or approval waiting on the user, in words, or null. */
    needsYou: string | null;
    /** A held/stopping/launch status in words (the row's statuses), or null. */
    held: string | null;
    /** The opening line of an external turn, while it applies, or null. */
    leadIn: string | null;
    activity: ActivityState | null;
    /** When this turn started (the ledger's, else the row's), for the plan's freshness. */
    turnStartedAt: number | null;
    /** The session's ambient summary, or null. */
    goal: string | null;
    /** The cycling phrase, without its ellipsis. */
    phrase: string;
}

export interface StatusLine {
    text: string;
    /** Shown after the text, muted, and truncated first: the session's goal,
     *  beside a line about what is happening right now (ranks 2–5). */
    detail?: string;
    rank: number;
    /** Same key: the same line, its counters moved. A new key types out. */
    key: string;
}

export interface StatusMemory {
    line: StatusLine;
    /** When this key was first shown. */
    since: number;
    /** When its candidate was last eligible (for HOLD). */
    liveAt: number;
}

function line(rank: number, text: string, key = text): StatusLine {
    return { rank, text, key: `${rank}:${key}` };
}

/** Rank 4: what is running, once it has lasted long enough. A subagent's
 *  own calls show under its Agent call ("Explore agent: map it · Reading
 *  a.ts"); one call shows its test progress, else its time once long. (comment-hygiene: allow) */
function nowLine(a: ActivityState, nowMs: number): StatusLine | null {
    const running = a.tools.filter((t) => t.activity.family !== "plan");
    const ids = new Set(running.map((t) => t.id).filter((id): id is string => id != null));
    const tools = running.filter((t) => !t.parentId || !ids.has(t.parentId));
    if (tools.length > 0) {
        const oldest = Math.min(...tools.map((t) => t.startedAt));
        if (nowMs - oldest < TIMING.toolPromoteMs) return null;
        const label = foldActivities(tools.map((t) => t.activity));
        if (!label) return null;
        const age = nowMs - oldest;
        if (tools.length === 1) {
            const only = tools[0];
            const step = running.filter((t) => t.parentId != null && t.parentId === only.id).at(-1);
            if (step) return line(RANK.now, `${label} · ${step.activity.label}`, label);
            if (only.progress) return line(RANK.now, `${label} · ${only.progress}`, label);
            if (age >= TIMING.showElapsedAfterMs) return line(RANK.now, `${label} · ${formatElapsedCompact(age)}`, label);
        }
        return line(RANK.now, label);
    }
    if (a.phase == null) return null;
    const age = nowMs - a.phaseSince;
    switch (a.phase) {
        case "thinking":
            if (age < TIMING.thinkingPromoteMs) return null;
            return a.thinkingHeadline ? line(RANK.now, `Thinking: ${a.thinkingHeadline}`) : line(RANK.now, "Thinking");
        case "writing":
            return age >= TIMING.writingPromoteMs ? line(RANK.now, "Writing the reply") : null;
        case "composing":
            return age >= TIMING.composingPromoteMs ? line(RANK.now, "Preparing the next step") : null;
        default:
            return null;
    }
}

/** Rank 2: a command that wrote output and then went quiet for a long while
 *  ("Running the build · no output for 2m"). Only commands that stream
 *  output at all: silence from one that never writes is not news. */
function quietLine(a: ActivityState, nowMs: number): StatusLine | null {
    const quiet = a.tools.find((t) => t.activity.family === "bash" && t.outputAt != null && nowMs - t.outputAt >= TIMING.quietToolMs);
    if (!quiet || quiet.outputAt == null) return null;
    return line(RANK.anomaly, `${quiet.activity.label} · no output for ${formatElapsedCompact(nowMs - quiet.outputAt)}`, `quiet:${quiet.id}`);
}

/** When a wait for the model counts as slow in this pane: SLOW_REQUEST, or
 *  twice its typical wait once it has enough of them (a model that is
 *  always slow shouldn't cry wolf every request). */
export function slowRequestMs(waits: readonly number[] | undefined): number {
    if (!waits || waits.length < 5) return TIMING.slowRequestMs;
    const sorted = [...waits].sort((x, y) => x - y);
    const median = sorted[Math.floor(sorted.length / 2)];
    return Math.max(TIMING.slowRequestMs, 2 * median);
}

/** Rank 2: a request still waiting for the model's first token. */
function anomalyLine(a: ActivityState, nowMs: number): StatusLine | null {
    const quiet = quietLine(a, nowMs);
    if (quiet) return quiet;
    if (a.phase !== "requesting" || a.tools.length > 0) return null;
    const age = nowMs - a.phaseSince;
    return age >= slowRequestMs(a.waits) ? line(RANK.anomaly, `Waiting on the model · ${formatElapsedCompact(age)}`, "waiting") : null;
}

/** Every line eligible now, best first. */
export function statusCandidates(input: StatusInput): StatusLine[] {
    const out: StatusLine[] = [];
    if (input.needsYou) out.push(line(RANK.needsYou, input.needsYou));
    if (input.held) out.push(line(RANK.held, input.held, input.held.replace(/\d+/g, "#")));
    const a = input.activity;
    if (a) {
        const anomaly = anomalyLine(a, input.nowMs);
        if (anomaly) out.push(anomaly);
    }
    if (input.leadIn) out.push(line(RANK.leadIn, input.leadIn));
    if (a) {
        const now = nowLine(a, input.nowMs);
        if (now) out.push(now);
        if (a.plan && input.turnStartedAt != null && a.planAt >= input.turnStartedAt) out.push(line(RANK.plan, planLine(a.plan)));
    }
    if (input.goal) out.push(line(RANK.goal, input.goal));
    out.push(line(RANK.phrase, `${input.phrase}…`, "phrase"));
    return out;
}

/** The goal beside a line about the moment (ranks 2–5), never beside itself
 *  or a status that owns the row. */
function withDetail(l: StatusLine, goal: string | null): StatusLine {
    return goal && l.rank >= RANK.anomaly && l.rank <= RANK.plan ? { ...l, detail: goal } : l;
}

/** The line to show now, and the memory to pass next time. */
export function presentStatus(input: StatusInput, memory: StatusMemory | null): { line: StatusLine; memory: StatusMemory } {
    const r = choose(input, memory);
    return { line: withDetail(r.line, input.goal), memory: r.memory };
}

function choose(input: StatusInput, memory: StatusMemory | null): { line: StatusLine; memory: StatusMemory } {
    const now = input.nowMs;
    const candidates = statusCandidates(input);
    const best = candidates[0];
    const remember = (chosen: StatusLine, eligible: boolean) => {
        const same = memory?.line.key === chosen.key;
        return {
            line: chosen,
            memory: { line: chosen, since: same && memory ? memory.since : now, liveAt: eligible ? now : (memory?.liveAt ?? now) },
        };
    };
    if (!memory || best.rank <= RANK.anomaly) return remember(best, true);

    // The line on screen, as it reads now if it is still eligible.
    const prev = candidates.find((c) => c.key === memory.line.key);
    if (prev) {
        // A same-or-lower rank waits out the dwell; a higher one comes now.
        if (best.key !== prev.key && best.rank >= prev.rank && now - memory.since < TIMING.dwellMs) return remember(prev, true);
        return remember(best, true);
    }
    // Its activity ended: a rank-4 line lingers briefly unless something at
    // least as specific is ready, so the row doesn't flash between two calls.
    if (memory.line.rank === RANK.now && best.rank > RANK.now && now - memory.liveAt < TIMING.holdMs) {
        return remember(memory.line, false);
    }
    return remember(best, true);
}
