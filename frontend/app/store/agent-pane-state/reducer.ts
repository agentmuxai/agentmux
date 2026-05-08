// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's lifecycle/turn/tool/tokens/stopping/
 * pending state. See docs/specs/agent-pane-document-reducer-2026-05-03.md
 * (slice #1 — pattern reference) and frontend-reducer-conventions-2026-05-03.md.
 *
 * Invariants enforced:
 *   1. turnActive cannot be set while streaming inactive (suppressed).
 *   2. turnActive cannot be set while initPhase !== "ready" (suppressed).
 *   3. stopping clears automatically on TurnEnd / TurnReset.
 *   4. currentTool and turnTokens clear on TurnEnd / TurnReset.
 *   5. pending FIFO; accepting/rejecting/expiring unknown ids is no-op.
 *   6. StreamFlushObserved is a no-op when streaming inactive.
 *   7. StreamWatchdogTick is a no-op when streaming inactive or
 *      lastEventMs is null (no events seen yet).
 */

import {
    AgentPaneCommand,
    AgentPaneState,
    ReducerResult,
    STUCK_THRESHOLD_MS,
} from "./types";

export function update(
    state: AgentPaneState,
    command: AgentPaneCommand,
    /**
     * Time injection for testability + watchdog determinism. Used by
     * tool/token transitions to bump `lastEventMs` (the stuck-stream
     * watchdog clock). Defaults to `Date.now()` so existing callers
     * don't need to thread a timestamp.
     */
    nowMs: number = Date.now(),
): ReducerResult {
    switch (command.type) {
        // ── Init lifecycle (gap 1) ─────────────────────────────────
        case "InitStart": {
            if (state.initPhase === "loading") return { state, events: [] };
            return {
                state: { ...state, initPhase: "loading", initError: null },
                events: [{ type: "init-started" }],
            };
        }

        case "InitReady": {
            if (state.initPhase === "ready") return { state, events: [] };
            return {
                state: { ...state, initPhase: "ready", initError: null },
                events: [{ type: "init-ready" }],
            };
        }

        case "InitFailed": {
            return {
                state: { ...state, initPhase: "error", initError: command.reason },
                events: [{ type: "init-failed", reason: command.reason }],
            };
        }

        // ── Stream lifecycle ───────────────────────────────────────
        case "StreamSubscribe":
            return {
                state: {
                    ...state,
                    streaming: {
                        ...state.streaming,
                        active: true,
                        lastEventTime: command.at,
                    },
                    lastEventMs: command.at,
                },
                events: [{ type: "stream-subscribed", at: command.at }],
            };

        case "StreamUnsubscribe":
            return {
                state: {
                    ...state,
                    streaming: { ...state.streaming, active: false },
                    // Defensive: subscription gone → no turn can be active.
                    turnActive: false,
                    lastEventMs: null,
                },
                events: [{ type: "stream-unsubscribed", at: command.at }],
            };

        case "StreamFlushObserved": {
            if (!state.streaming.active) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    streaming: {
                        ...state.streaming,
                        bufferSize: state.streaming.bufferSize + command.addedCount,
                        lastEventTime: command.at,
                    },
                    lastEventMs: command.at,
                },
                events: [
                    { type: "stream-flush-observed", addedCount: command.addedCount },
                ],
            };
        }

        case "StreamWatchdogTick": {
            // No-op when stream inactive or no event has ever been seen.
            if (!state.streaming.active || state.lastEventMs == null) {
                return { state, events: [] };
            }
            const idleSinceMs = command.nowMs - state.lastEventMs;
            if (idleSinceMs < STUCK_THRESHOLD_MS) {
                return { state, events: [] };
            }
            return {
                state,
                events: [
                    {
                        type: "stream-stuck",
                        idleSinceMs,
                        thresholdMs: STUCK_THRESHOLD_MS,
                    },
                ],
            };
        }

        case "TurnStart": {
            // Invariant 1: can't start a turn without an active stream.
            if (!state.streaming.active) {
                return {
                    state,
                    events: [
                        {
                            type: "turn-start-suppressed",
                            reason: "stream not active",
                        },
                    ],
                };
            }
            // Invariant 2: can't start a turn while still loading history.
            // After init failure we fail open — the live stream may still
            // work even if history fetch hung, and blocking the user
            // would be worse than silently dropping a stale send.
            if (state.initPhase === "loading") {
                return {
                    state,
                    events: [
                        {
                            type: "turn-start-suppressed",
                            reason: "init still loading",
                        },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    turnActive: true,
                    sessionStats: null, // clear stale stats from prior turn
                    lastEventMs: command.at,
                },
                events: [{ type: "turn-started", at: command.at }],
            };
        }

        case "TurnEnd": {
            const stoppingWasSet = state.stopping;
            // Merge stats with live turn-tokens (mirrors prior finalizeTurn
            // logic — see PR #549 reagent/codex P1 reference).
            const merged = mergeStats(command.stats, state.turnTokens);
            return {
                state: {
                    ...state,
                    sessionStats: merged,
                    currentTool: null,
                    turnTokens: null,
                    turnActive: false,
                    stopping: false,
                },
                events: [
                    {
                        type: "turn-ended",
                        statsMerged: merged != null,
                        stoppingCleared: stoppingWasSet,
                    },
                ],
            };
        }

        case "TurnReset":
            return {
                state: {
                    ...state,
                    sessionStats: null,
                    currentTool: null,
                    turnTokens: null,
                    turnActive: false,
                    // stopping is part of turn state — clear it too.
                    stopping: false,
                },
                events: [{ type: "turn-reset" }],
            };

        case "ToolStart":
            return {
                state: bumpEvent({ ...state, currentTool: command.name }, nowMs),
                events: [{ type: "tool-started", name: command.name }],
            };

        case "ToolEnd":
            return {
                state: bumpEvent({ ...state, currentTool: null }, nowMs),
                events: [{ type: "tool-ended" }],
            };

        case "TokensIn": {
            const next = {
                input: command.input,
                output: state.turnTokens?.output ?? 0,
            };
            return {
                state: bumpEvent({ ...state, turnTokens: next }, nowMs),
                events: [{ type: "tokens-updated", input: command.input, output: null }],
            };
        }

        case "TokensOut": {
            const next = {
                input: state.turnTokens?.input ?? 0,
                output: command.output,
            };
            return {
                state: bumpEvent({ ...state, turnTokens: next }, nowMs),
                events: [{ type: "tokens-updated", input: null, output: command.output }],
            };
        }

        case "RequestStop":
            return {
                state: { ...state, stopping: true },
                events: [{ type: "stop-requested", at: command.at }],
            };

        case "StopFailed":
            return {
                state: { ...state, stopping: false },
                events: [{ type: "stop-failed" }],
            };

        case "PendingMessageQueued":
            return {
                state: {
                    ...state,
                    pending: [
                        ...state.pending,
                        { id: command.id, text: command.text, createdAt: command.at },
                    ],
                },
                events: [{ type: "pending-queued", id: command.id }],
            };

        case "PendingMessageAccepted": {
            const wasPresent = state.pending.some((m) => m.id === command.id);
            if (!wasPresent) {
                return {
                    state,
                    events: [{ type: "pending-accepted", id: command.id, wasPresent: false }],
                };
            }
            return {
                state: {
                    ...state,
                    pending: state.pending.filter((m) => m.id !== command.id),
                },
                events: [{ type: "pending-accepted", id: command.id, wasPresent: true }],
            };
        }

        case "PendingMessageRejected": {
            const wasPresent = state.pending.some((m) => m.id === command.id);
            if (!wasPresent) {
                return {
                    state,
                    events: [{ type: "pending-rejected", id: command.id, wasPresent: false }],
                };
            }
            return {
                state: {
                    ...state,
                    pending: state.pending.filter((m) => m.id !== command.id),
                },
                events: [{ type: "pending-rejected", id: command.id, wasPresent: true }],
            };
        }

        case "PendingMessageExpired": {
            const entry = state.pending.find((m) => m.id === command.id);
            if (!entry) {
                return {
                    state,
                    events: [
                        {
                            type: "pending-expired",
                            id: command.id,
                            queuedAt: 0,
                            ageMs: 0,
                            wasPresent: false,
                        },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    pending: state.pending.filter((m) => m.id !== command.id),
                },
                events: [
                    {
                        type: "pending-expired",
                        id: command.id,
                        queuedAt: entry.createdAt,
                        ageMs: nowMs - entry.createdAt,
                        wasPresent: true,
                    },
                ],
            };
        }
    }
}

/**
 * Bump `lastEventMs` to "now" only while the stream is subscribed. Used
 * by tool/token transitions which are observable stream activity. Skip
 * the bump when streaming is inactive — those would correspond to stale
 * commands and shouldn't reset the watchdog clock.
 */
function bumpEvent(state: AgentPaneState, nowMs: number): AgentPaneState {
    if (!state.streaming.active) return state;
    return { ...state, lastEventMs: nowMs };
}

/**
 * Mirror of the prior finalizeTurn merge: prefer the explicit stats from
 * the result event, but inject live turn-token totals because the result
 * event sometimes lacks them.
 */
function mergeStats(
    stats: AgentPaneState["sessionStats"],
    tokens: AgentPaneState["turnTokens"],
): AgentPaneState["sessionStats"] {
    if (stats) {
        return { ...stats, input_tokens: tokens?.input, output_tokens: tokens?.output };
    }
    if (tokens) {
        return { input_tokens: tokens.input, output_tokens: tokens.output };
    }
    return null;
}
