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
    KindBeforeDisconnect,
    ReducerResult,
    STUCK_THRESHOLD_MS,
    TurnOutcome,
    TurnPhase,
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
        case "StreamSubscribe": {
            // Dual-write: if a turn was Submitting, the subscription is
            // its expected hand-off to Streaming. Otherwise the phase
            // stays as-is (subscribing without a pending send isn't
            // "working"; Idle/Done/Disconnected/Streaming stay put).
            const nextPhase: TurnPhase =
                state.turnPhase.kind === "Submitting"
                    ? {
                          kind: "Streaming",
                          bufferSize: state.streaming.bufferSize,
                          toolsActive: 0,
                          lastEventMs: command.at,
                      }
                    : state.turnPhase;
            return {
                state: {
                    ...state,
                    streaming: {
                        ...state.streaming,
                        active: true,
                        lastEventTime: command.at,
                    },
                    lastEventMs: command.at,
                    turnPhase: nextPhase,
                },
                events: [{ type: "stream-subscribed", at: command.at }],
            };
        }

        case "StreamUnsubscribe": {
            // Dual-write: if we were working (Submitting/Streaming/
            // Interrupting), surface a Disconnected phase so the view
            // can show re-attach UX. Otherwise → Idle.
            const k = state.turnPhase.kind;
            const wasWorking =
                k === "Submitting" || k === "Streaming" || k === "Interrupting";
            const nextPhase: TurnPhase = wasWorking
                ? {
                      kind: "Disconnected",
                      lastKind: k as KindBeforeDisconnect,
                      reason: "stream-unsubscribed",
                  }
                : { kind: "Idle" };
            return {
                state: {
                    ...state,
                    streaming: { ...state.streaming, active: false },
                    // Defensive: subscription gone → no turn can be active.
                    turnActive: false,
                    lastEventMs: null,
                    turnPhase: nextPhase,
                },
                events: [{ type: "stream-unsubscribed", at: command.at }],
            };
        }

        case "StreamFlushObserved": {
            if (!state.streaming.active) {
                return { state, events: [] };
            }
            const newBuf = state.streaming.bufferSize + command.addedCount;
            // Dual-write: while Streaming, mirror bufferSize + lastEventMs.
            // From Submitting, PROMOTE — a flush is the first concrete
            // "stream is producing" signal after a turn submit. codex P1
            // on #987 (companion to the bumpEvent promotion). Other
            // phases (Idle, Done, Disconnected, Interrupting) keep their
            // shape — flushes outside a turn are ambient.
            const nextPhase: TurnPhase =
                state.turnPhase.kind === "Streaming"
                    ? {
                          ...state.turnPhase,
                          bufferSize: newBuf,
                          lastEventMs: command.at,
                      }
                    : state.turnPhase.kind === "Submitting"
                        ? {
                              kind: "Streaming",
                              bufferSize: newBuf,
                              toolsActive: 0,
                              lastEventMs: command.at,
                          }
                        : state.turnPhase;
            return {
                state: {
                    ...state,
                    streaming: {
                        ...state.streaming,
                        bufferSize: newBuf,
                        lastEventTime: command.at,
                    },
                    lastEventMs: command.at,
                    turnPhase: nextPhase,
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
                    // Dual-write: Submitting until first stream event /
                    // subscribe transitions us to Streaming. PR A doesn't
                    // know the pendingContent (TurnStart payload doesn't
                    // carry it); a later PR can thread that through.
                    turnPhase: {
                        kind: "Submitting",
                        submittedAt: command.at,
                        pendingContent: "",
                    },
                },
                events: [{ type: "turn-started", at: command.at }],
            };
        }

        case "TurnEnd": {
            const stoppingWasSet = state.stopping;
            // Merge stats with live turn-tokens (mirrors prior finalizeTurn
            // logic — see PR #549 reagent/codex P1 reference).
            const merged = mergeStats(command.stats, state.turnTokens);
            // Dual-write outcome: stop-in-flight → "stopped"; otherwise
            // "completed". "interrupted" / "errored" are reserved for
            // future-PR commands (StreamStalled, Disconnected, etc.).
            const outcome: TurnOutcome = stoppingWasSet ? "stopped" : "completed";
            return {
                state: {
                    ...state,
                    sessionStats: merged,
                    currentTool: null,
                    turnTokens: null,
                    turnActive: false,
                    stopping: false,
                    turnPhase: {
                        kind: "Done",
                        outcome,
                        finishedAt: nowMs,
                    },
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
                    // Dual-write: TurnReset is a wholesale clear → Idle.
                    turnPhase: { kind: "Idle" },
                },
                events: [{ type: "turn-reset" }],
            };

        case "ToolStart":
            return {
                state: bumpEvent(
                    { ...state, currentTool: command.name },
                    nowMs,
                    /* toolsDelta */ +1,
                ),
                events: [{ type: "tool-started", name: command.name }],
            };

        case "ToolEnd":
            return {
                state: bumpEvent(
                    { ...state, currentTool: null },
                    nowMs,
                    /* toolsDelta */ -1,
                ),
                events: [{ type: "tool-ended" }],
            };

        case "TokensIn": {
            const next = {
                input: command.input,
                output: state.turnTokens?.output ?? 0,
            };
            return {
                state: bumpEvent({ ...state, turnTokens: next }, nowMs, 0),
                events: [{ type: "tokens-updated", input: command.input, output: null }],
            };
        }

        case "TokensOut": {
            const next = {
                input: state.turnTokens?.input ?? 0,
                output: command.output,
            };
            return {
                state: bumpEvent({ ...state, turnTokens: next }, nowMs, 0),
                events: [{ type: "tokens-updated", input: null, output: command.output }],
            };
        }

        case "RequestStop": {
            // Dual-write: if a turn is in flight, surface Interrupting;
            // otherwise keep the phase as-is (stopping is set but there
            // is no real working state — the legacy boolean alone is
            // enough until the view migrates in PR B).
            const k = state.turnPhase.kind;
            const isWorking =
                k === "Submitting" || k === "Streaming" || k === "Interrupting";
            const nextPhase: TurnPhase = isWorking
                ? {
                      kind: "Interrupting",
                      reason: "user",
                      sigintSentAt: command.at,
                  }
                : state.turnPhase;
            return {
                state: { ...state, stopping: true, turnPhase: nextPhase },
                events: [{ type: "stop-requested", at: command.at }],
            };
        }

        case "StopFailed": {
            // Dual-write: rollback Interrupting → previous working kind
            // best-effort. We don't track the prior kind on Interrupting,
            // so use the stream's liveness as the deterministic signal:
            //   streaming.active  → Streaming (the most common case)
            //   else              → Idle (no longer in a turn)
            const k = state.turnPhase.kind;
            const nextPhase: TurnPhase =
                k === "Interrupting"
                    ? state.streaming.active
                        ? {
                              kind: "Streaming",
                              bufferSize: state.streaming.bufferSize,
                              toolsActive: 0,
                              lastEventMs: state.lastEventMs ?? nowMs,
                          }
                        : { kind: "Idle" }
                    : state.turnPhase;
            return {
                state: { ...state, stopping: false, turnPhase: nextPhase },
                events: [{ type: "stop-failed" }],
            };
        }

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
 *
 * PR A dual-write: if the phase is Streaming, also update its
 * `lastEventMs` and apply `toolsDelta` to `toolsActive` (clamped ≥ 0).
 */
function bumpEvent(
    state: AgentPaneState,
    nowMs: number,
    toolsDelta: number,
): AgentPaneState {
    if (!state.streaming.active) return state;
    const next: AgentPaneState = { ...state, lastEventMs: nowMs };
    if (next.turnPhase.kind === "Streaming") {
        next.turnPhase = {
            ...next.turnPhase,
            lastEventMs: nowMs,
            toolsActive: Math.max(0, next.turnPhase.toolsActive + toolsDelta),
        };
    } else if (next.turnPhase.kind === "Submitting") {
        // codex P1 on #987: `StreamSubscribe` fires once at mount, so
        // subsequent turns enter Submitting and then no transition
        // arm ever advances them to Streaming — chunks/tokens/tools
        // arrive but phase stayed stuck. First stream activity from
        // Submitting is the actual "stream is producing" signal;
        // promote here so the dual-written phase reflects reality.
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
