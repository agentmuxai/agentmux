// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneState, ReducerResult } from "./types";
import { implausibleReason, makeContextReading, mergeContextWindows, plausibleReading, rewindowContextReading } from "./context-reading";
import { rejectedEvent, withReading } from "./context-reading-state";

/** `ReconcileContextFromHistory`. */
export function onReconcileContextFromHistory(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ReconcileContextFromHistory" }>): ReducerResult {
    // The history's reported windows are older than any reported
    // live, so live entries stay on top.
    const reportedContextWindows = command.reportedWindows
        ? mergeContextWindows(command.reportedWindows, state.reportedContextWindows)
        : state.reportedContextWindows;
    const merged = { ...state, reportedContextWindows };
    const known = { reported: reportedContextWindows, learned: state.learnedContextWindows };
    // Seeds only while nothing more current has spoken — a live
    // reading, an invalidation, a compaction, a reset — never over
    // real-time data, and never a conversation invalidated while the
    // history was still loading (`contextSeedable`). The windows
    // still merge either way.
    if (!state.contextSeedable || command.tokens == null) {
        if (reportedContextWindows === state.reportedContextWindows) return { state, events: [] };
        return withReading(merged, rewindowContextReading(state.context, known), plausibleReading(state.context) != null);
    }
    const model = command.model ?? state.lastContextModel;
    const reading = makeContextReading(
        { tokens: command.tokens, model, source: "history", at: command.at ?? null },
        known,
    );
    const reason = implausibleReason(reading);
    if (reason != null) {
        return {
            state: reportedContextWindows === state.reportedContextWindows ? state : merged,
            events: [rejectedEvent(reading, reason)],
        };
    }
    const seeded = withReading(
        { ...merged, contextSeedable: false, lastContextModel: state.lastContextModel ?? model },
        reading,
        false,
    );
    return {
        state: seeded.state,
        events: [{ type: "context-reconciled-at-mount", tokens: command.tokens }, ...seeded.events],
    };
}

/** `ContextWindowsReported`. */
export function onContextWindowsReported(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ContextWindowsReported" }>): ReducerResult {
    const reportedContextWindows = mergeContextWindows(state.reportedContextWindows, command.windows);
    if (reportedContextWindows === state.reportedContextWindows) return { state, events: [] };
    const reading = rewindowContextReading(state.context, {
        reported: reportedContextWindows,
        learned: state.learnedContextWindows,
    });
    return withReading({ ...state, reportedContextWindows }, reading, plausibleReading(state.context) != null);
}

/** `ContextReported`. */
export function onContextReported(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ContextReported" }>, nowMs: number): ReducerResult {
    const model = command.model ?? state.lastContextModel; // and the window it was measured against:
    const said = command.window != null && model != null ? { [model]: command.window } : null;
    const reportedContextWindows = said ? mergeContextWindows(state.reportedContextWindows, said) : state.reportedContextWindows;
    const known = { reported: reportedContextWindows, learned: state.learnedContextWindows };
    const reading = makeContextReading({ tokens: command.tokens, model, source: "live", at: nowMs }, known);
    const next = { ...state, reportedContextWindows, lastContextModel: model, contextSeedable: false };
    return withReading(next, reading, plausibleReading(state.context) != null);
}

/** `ContextInvalidated`. */
export function onContextInvalidated(state: AgentPaneState): ReducerResult {
    if (state.context == null && !state.contextSeedable) return { state, events: [] };
    return { state: { ...state, context: null, contextSeedable: false }, events: [] };
}

/** `ContextModelSwitched`. */
export function onContextModelSwitched(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ContextModelSwitched" }>): ReducerResult {
    const r = state.context;
    if (r == null || r.switchedTo === command.model) return { state, events: [] };
    // Switching back to the model the reading was measured on before
    // any reply: its window holds again.
    if (r.model != null && command.model === r.model) {
        const back = rewindowContextReading(
            { ...r, switchedTo: null },
            { reported: state.reportedContextWindows, learned: state.learnedContextWindows },
        );
        return { state: { ...state, context: back }, events: [] };
    }
    return {
        state: { ...state, context: { ...r, window: null, windowSource: null, switchedTo: command.model } },
        events: [],
    };
}
