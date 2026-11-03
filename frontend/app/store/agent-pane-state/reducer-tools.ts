// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneEvent, AgentPaneState, COMPACTION_HEURISTIC_SUPPRESS_MS, ReducerResult, type ModelActivityPhase } from "./types";
import { makeContextReading, plausibleReading } from "./context-reading";
import { withReading } from "./context-reading-state";
import { outputSoFar, turnOutputTokens, withShownOutput } from "./turn-contribution";
import { withWaitRecorded, withPhase, activityWithTool, activityWithoutTool, ledgerStamp, bumpEvent } from "./reducer-helpers";

/** `ToolStart`. */
export function onToolStart(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ToolStart" }>, nowMs: number): ReducerResult {
    const nextState = bumpEvent(
        {
            ...state,
            currentTool: command.name,
            currentToolArg: command.arg ?? null,
            activity: activityWithTool(state.activity, command, nowMs),
        },
        nowMs,
        /* toolsDelta */ +1,
    );
    const events: AgentPaneEvent[] = [
        { type: "tool-started", name: command.name },
    ];
    return { state: nextState, events };
}

/** `ToolEnd`. */
export function onToolEnd(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ToolEnd" }>, nowMs: number): ReducerResult {
    const nextState = bumpEvent(
        { ...state, currentTool: null, currentToolArg: null, activity: activityWithoutTool(state.activity, command.id) },
        nowMs,
        /* toolsDelta */ -1,
    );
    const events: AgentPaneEvent[] = [{ type: "tool-ended" }];
    return { state: nextState, events };
}

/** `TokensIn`. */
export function onTokensIn(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "TokensIn" }>, nowMs: number): ReducerResult {
    const prevReading = state.context;
    // A new call (the stream counts each call once, by message id):
    // the previous call's output is final, so it moves into
    // `outputDone`, and the new call starts streaming (↓).
    const prevTokens = state.turnTokens;
    const next = withShownOutput({
        input: command.input,
        output: 0,
        freshInput: command.freshInput,
        cacheCreation: command.cacheCreation,
        cacheRead: command.cacheRead,
        outputDone: prevTokens ? outputSoFar(prevTokens) : 0,
        streamedChars: 0,
        requesting: false,
        shownOutput: prevTokens?.shownOutput,
        ...ledgerStamp(state),
    });
    // The window: reported by Claude Code for this model, a larger one
    // proven by an accepted prompt, else the model-name table
    // (context-reading.ts). Unknown stays unknown.
    const model = command.model ?? state.lastContextModel;
    const reading = makeContextReading(
        { tokens: command.input, model, source: "live", at: nowMs },
        { reported: state.reportedContextWindows, learned: state.learnedContextWindows },
    );
    // Kept in state even when the meter won't show it (the next call
    // replaces it). Refusing it is news only when the meter had
    // something to show, or nothing at all, before — not on every
    // call of a turn that keeps reporting the same impossibility.
    const withLive = withReading(
        {
            ...state,
            turnTokens: next,
            lastContextModel: model,
            contextSeedable: false,
            activity: { ...withPhase(withWaitRecorded(state.activity, nowMs), "responding", nowMs), thinkingHeadline: undefined },
        },
        reading,
        prevReading == null || plausibleReading(prevReading) != null,
    );
    const nextState = bumpEvent(withLive.state, nowMs, 0);
    const events: AgentPaneEvent[] = [
        { type: "tokens-updated", input: command.input, output: null },
        ...withLive.events,
    ];
    // Detect context compaction: token count drops ≥50% from a
    // non-trivial baseline. Compaction typically drops 80–95%;
    // normal turn-to-turn growth is monotonically increasing.
    // AgentMux /clear is frontend-only and does not affect tokens.
    //
    // Backstop only for Claude: a REAL `CompactionBoundary` event
    // (exact backend data, not inferred) already fired its own
    // `context-compacted` and cleared `context` (so there is no
    // "before" to compare) — the window below suppresses the
    // heuristic as well, so the same boundary can't produce two
    // events whatever the ordering.
    // Providers without a structured signal (codex/gemini/copilot)
    // never set `lastCompactionBoundaryAt`, so the heuristic stays
    // fully active for them. See
    // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
    //
    // Only a reading this process measured counts as "before": a
    // seed restored from history describes a session that may since
    // have been resumed, replaced or compacted out of sight, and
    // comparing against it once produced a "compacted 17m → 300k"
    // card on a pane's first turn (REPORT_AGENT_PANE_CONTEXT_METER_
    // 2026_10_05.md §2.3). An implausible reading is no baseline either.
    const prevPlausible = plausibleReading(prevReading);
    const prev = prevPlausible?.source === "live" ? prevPlausible.tokens : null;
    const suppressedByRealBoundary =
        state.lastCompactionBoundaryAt != null
        && nowMs - state.lastCompactionBoundaryAt < COMPACTION_HEURISTIC_SUPPRESS_MS;
    if (!suppressedByRealBoundary && prev != null && prev > 10_000 && command.input < prev * 0.5) {
        events.push({
            type: "context-compacted",
            tokensBefore: prev,
            tokensAfter: command.input,
            source: "heuristic",
        });
    }
    return { state: nextState, events };
}

/** `ModelEndedTurn and ModelMessageStarted`. */
export function onModelEndedTurn(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ModelEndedTurn" | "ModelMessageStarted" }>): ReducerResult {
    // Deliberately not a liveness event (no bumpEvent): the stream
    // lines carrying these already refresh liveness through the
    // flush path. See `modelEndedTurn` on the Streaming phase.
    const phase = state.turnPhase;
    if (phase.kind !== "Streaming") return { state, events: [] };
    const ended = command.type === "ModelEndedTurn";
    if ((phase.modelEndedTurn ?? false) === ended) return { state, events: [] };
    return { state: { ...state, turnPhase: { ...phase, modelEndedTurn: ended } }, events: [] };
}

/** `TokensOut`. */
export function onTokensOut(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "TokensOut" }>, nowMs: number): ReducerResult {
    // The latest call's exact output (message_delta, at the call's
    // end): it replaces the call's streamed estimate. Everything else
    // carries over from its TokensIn.
    const next = withShownOutput({
        ...state.turnTokens,
        input: state.turnTokens?.input ?? 0,
        output: command.output,
        streamedChars: 0,
    });
    const nextState = bumpEvent(
        { ...state, turnTokens: next },
        nowMs,
        0,
    );
    const events: AgentPaneEvent[] = [
        { type: "tokens-updated", input: null, output: command.output },
    ];
    return { state: nextState, events };
}

// The working row's counter (turnOutputTokens): an output estimate for
// the call still streaming, and the ↑/↓ phase. Not liveness events:
// the same lines already reach the reducer as StreamFlushObserved.
// Before the turn's first call there is nothing to count yet.
export function onToolProgress(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ToolProgress" }>): ReducerResult {
    const i = state.activity.tools.findIndex((t) => t.id === command.id);
    if (i < 0 || state.activity.tools[i].progress === command.text) return { state, events: [] };
    const tools = state.activity.tools.map((t, j) => (j === i ? { ...t, progress: command.text } : t));
    return { state: { ...state, activity: { ...state.activity, tools } }, events: [] };
}

/** `ToolOutput`. */
export function onToolOutput(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ToolOutput" }>): ReducerResult {
    const i = state.activity.tools.findIndex((t) => t.id === command.id);
    if (i < 0) return { state, events: [] };
    const tools = state.activity.tools.map((t, j) => (j === i ? { ...t, outputAt: command.at } : t));
    return { state: { ...state, activity: { ...state.activity, tools } }, events: [] };
}

/** `ThinkingHeadline`. */
export function onThinkingHeadline(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ThinkingHeadline" }>): ReducerResult {
    if (state.activity.thinkingHeadline === command.text) return { state, events: [] };
    return { state: { ...state, activity: { ...state.activity, thinkingHeadline: command.text } }, events: [] };
}

/** `OutputStreamed`. */
export function onOutputStreamed(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "OutputStreamed" }>, nowMs: number): ReducerResult {
    const t = state.turnTokens;
    if (!t || command.chars <= 0) return { state, events: [] };
    const next = withShownOutput({ ...t, streamedChars: (t.streamedChars ?? 0) + command.chars, requesting: false });
    const phase: ModelActivityPhase | null =
        command.kind === "thinking" ? "thinking" : command.kind === "text" ? "writing" : command.kind === "tool_input" ? "composing" : null;
    const activity = phase ? withPhase(state.activity, phase, nowMs) : state.activity;
    return { state: { ...state, turnTokens: next, activity }, events: [] };
}
