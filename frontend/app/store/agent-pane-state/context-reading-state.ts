// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Setting the pane's context reading, and the events that warrants (reducer.ts).

import { implausibleReason, learnedWindowsAfter, refutedReportedWindow, windowFor, type ContextReading } from "./context-reading";
import type { AgentPaneEvent, AgentPaneState } from "./types";

/** The `context-reading-rejected` event for a reading `implausibleReason` refused. */
export function rejectedEvent(reading: ContextReading, reason: string): AgentPaneEvent {
    return {
        type: "context-reading-rejected",
        tokens: reading.tokens,
        window: reading.window,
        model: reading.model,
        source: reading.source,
        reason,
    };
}

/**
 * `state` with `reading` as the pane's reading — any window it proved recorded
 * per model — plus the events it warrants: `context-reading-rejected` when it
 * can't be shown and `wasShown` says it is news (a reading that just turned
 * implausible, not one refused again), and `context-window-refuted` when it
 * newly contradicts a reported window.
 */
export function withReading(
    state: AgentPaneState,
    reading: ContextReading | null,
    wasShown: boolean,
): { state: AgentPaneState; events: AgentPaneEvent[] } {
    const events: AgentPaneEvent[] = [];
    if (reading) {
        const reason = implausibleReason(reading);
        if (reason != null && wasShown) events.push(rejectedEvent(reading, reason));
        const refuted = refutedReportedWindow(reading, state.reportedContextWindows);
        const knownBefore = windowFor(state.learnedContextWindows, reading.model);
        if (refuted != null && reading.model && reading.window != null && (knownBefore == null || knownBefore < reading.window)) {
            events.push({
                type: "context-window-refuted",
                model: reading.model,
                reported: refuted,
                tokens: reading.tokens,
                window: reading.window,
            });
        }
    }
    const learnedContextWindows = learnedWindowsAfter(state.learnedContextWindows, reading);
    return { state: { ...state, context: reading, learnedContextWindows }, events };
}
