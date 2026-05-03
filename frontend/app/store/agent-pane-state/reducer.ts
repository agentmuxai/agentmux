// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's lifecycle/turn/tool/tokens/stopping/
 * pending state. See docs/specs/agent-pane-document-reducer-2026-05-03.md
 * (slice #1 — pattern reference) and frontend-reducer-conventions-2026-05-03.md.
 *
 * Invariants enforced:
 *   1. turnActive cannot be set while streaming inactive (suppressed).
 *   2. stopping clears automatically on TurnEnd / TurnReset.
 *   3. currentTool and turnTokens clear on TurnEnd / TurnReset.
 *   4. pending FIFO; accepting/rejecting unknown ids is idempotent no-op.
 *   5. StreamFlushObserved is a no-op when streaming inactive.
 */

import {
    AgentPaneCommand,
    AgentPaneState,
    ReducerResult,
} from "./types";

export function update(
    state: AgentPaneState,
    command: AgentPaneCommand,
): ReducerResult {
    switch (command.type) {
        case "StreamSubscribe":
            return {
                state: {
                    ...state,
                    streaming: {
                        ...state.streaming,
                        active: true,
                        lastEventTime: command.at,
                    },
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
                },
                events: [
                    { type: "stream-flush-observed", addedCount: command.addedCount },
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
            return {
                state: {
                    ...state,
                    turnActive: true,
                    sessionStats: null, // clear stale stats from prior turn
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
                state: { ...state, currentTool: command.name },
                events: [{ type: "tool-started", name: command.name }],
            };

        case "ToolEnd":
            return {
                state: { ...state, currentTool: null },
                events: [{ type: "tool-ended" }],
            };

        case "TokensIn": {
            const next = {
                input: command.input,
                output: state.turnTokens?.output ?? 0,
            };
            return {
                state: { ...state, turnTokens: next },
                events: [{ type: "tokens-updated", input: command.input, output: null }],
            };
        }

        case "TokensOut": {
            const next = {
                input: state.turnTokens?.input ?? 0,
                output: command.output,
            };
            return {
                state: { ...state, turnTokens: next },
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
    }
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
