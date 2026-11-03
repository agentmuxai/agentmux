// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneState, ReducerResult } from "./types";

/** `PendingMessageFlushStarted`. */
export function onPendingMessageFlushStarted(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "PendingMessageFlushStarted" }>): ReducerResult {
    // Dispatched from useAgentCommands.ts's flushHeldMessages, right
    // before the actual deliverToBackend call — i.e. the moment
    // delivery genuinely begins, not merely "the turn that was
    // queued behind has ended." Codex P2 on PR #2970: an earlier
    // version of this fix set `flushing` unconditionally on TurnEnd,
    // which is wrong when flushHeldMessages bails without draining
    // (a still-pending controller refresh, useAgentCommands.ts
    // ~1433-1455) — the panel would claim "Sending — reaching the
    // agent any moment" (and hide the recall guidance) for a message
    // that hadn't actually started sending, possibly for as long as
    // the deferred refresh takes. Tying this to the real delivery
    // start instead means the entry only ever reads "Sending" when
    // that's genuinely true. No-op if the id isn't found (already
    // accepted/rejected/expired) or is already flushing.
    const entry = state.pending.find((m) => m.id === command.id);
    if (!entry || entry.flushing) return { state, events: [] };
    return {
        state: {
            ...state,
            pending: state.pending.map((m) =>
                m.id === command.id ? { ...m, flushing: true } : m,
            ),
        },
        events: [{ type: "pending-flush-started", id: command.id }],
    };
}

/** `PendingMessageQueued`. */
export function onPendingMessageQueued(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "PendingMessageQueued" }>): ReducerResult {
    return {
        state: {
            ...state,
            pending: [
                ...state.pending,
                {
                    id: command.id,
                    text: command.text,
                    createdAt: command.at,
                    enqueuedWhileBusy: command.enqueuedWhileBusy,
                    ...(command.attachments?.length ? { attachments: command.attachments } : {}), ...(command.system ? { system: true } : {}),
                },
            ],
        },
        events: [{ type: "pending-queued", id: command.id }],
    };
}

/** `PendingMessageAccepted`. */
export function onPendingMessageAccepted(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "PendingMessageAccepted" }>): ReducerResult {
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

/** `PendingMessageRejected`. */
export function onPendingMessageRejected(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "PendingMessageRejected" }>): ReducerResult {
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

/** `PendingMessageExpired`. */
export function onPendingMessageExpired(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "PendingMessageExpired" }>, nowMs: number): ReducerResult {
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

// ── Composer details / Log panel ──────────────────────────
export function onProviderWaiting(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ProviderWaiting" }>): ReducerResult {
    if (state.lastEventMs == null) return { state, events: [] };
    const next: AgentPaneState = { ...state, lastEventMs: command.at };
    if (next.turnPhase.kind === "Streaming") {
        next.turnPhase = {
            ...next.turnPhase,
            lastEventMs: command.at,
            waitingReason: command.reason,
            retryAfterMs: command.retryAfterMs,
            retryAtMs: command.retryAfterMs != null ? command.at + command.retryAfterMs : null,
        };
    }
    return {
        state: next,
        events: [{ type: "provider-waiting", reason: command.reason }],
    };
}
