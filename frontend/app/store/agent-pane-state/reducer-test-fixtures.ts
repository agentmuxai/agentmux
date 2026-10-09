// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// States the pane reducer's tests start from (reducer.*.test.ts).

import { update } from "./reducer";
import type { ContextReading } from "./context-reading";
import { initialState } from "./types";

/** Bring a fresh state into a live `Streaming` turn (toolsActive 0). */
export const streaming = (atMs = 100) => {
    const s1 = update(ready(atMs), { type: "TurnStart", at: atMs }).state;
    return update(s1, { type: "StreamFlushObserved", addedCount: 1, at: atMs }).state;
};

export const mk = () => initialState("test-agent");
/** A state whose context reading is `tokens`, measured live (no model). */
export const liveReading = (tokens: number, extra: Partial<ContextReading> = {}): ContextReading => ({
    tokens,
    model: null,
    window: null,
    windowSource: null,
    source: "live",
    at: 1,
    switchedTo: null,
    ...extra,
});
/**
 * Convenience: bring a fresh state up to the "ready + subscribed" baseline
 * that most turn-related tests assume. Issue #728 introduced an init-phase
 * gate, so subscribing alone no longer permits `TurnStart`.
 */
export const ready = (atMs = 100) => {
    const s0 = update(mk(), { type: "InitReady", at: atMs }).state;
    return update(s0, { type: "StreamSubscribe", at: atMs }).state;
};
