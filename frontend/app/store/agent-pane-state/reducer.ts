// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's lifecycle/turn/tool/tokens/stopping/
 * pending state. See docs/specs/agent-pane-document-reducer-2026-05-03.md
 * (slice #1 — pattern reference) and frontend-reducer-conventions-2026-05-03.md.
 *
 * Invariants enforced:
 *   1. turnActive cannot be set while streaming inactive (suppressed).
 *   2. turnActive cannot be set while initPhase.kind === "InitPending"
 *      (suppressed). After `InitFailed` we fail open — see the
 *      `TurnStart` arm.
 *   3. stopping clears automatically on TurnEnd / TurnReset.
 *   4. currentTool and turnTokens clear on TurnEnd / TurnReset.
 *   5. pending FIFO; accepting/rejecting/expiring unknown ids is no-op.
 *   6. StreamFlushObserved is a no-op when streaming inactive.
 *   7. StreamWatchdogTick is a no-op when streaming inactive or
 *      lastEventMs is null (no events seen yet).
 *   8. Init lifecycle transitions are one-way: InitStart / InitReady /
 *      InitFailed are no-ops once the pane has reached a terminal init
 *      state (InitReady / InitFailed). Re-entering pending requires a
 *      fresh pane slot.
 */

import {
    AgentPaneCommand,
    AgentPaneEvent,
    AgentPaneState,
    INTERRUPT_TIMEOUT_MS,
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
            // Idempotent: only emits when first entering pending. Already
            // in pending → no-op (same ref). Already terminal → no-op
            // (we don't reset back from InitReady/InitFailed).
            return { state, events: [] };
        }

        case "InitReady": {
            if (state.initPhase.kind !== "InitPending") {
                // Already terminal — no-op. Out-of-order InitReady after
                // an earlier InitFailed (or a duplicate InitReady) is
                // silently dropped to keep transitions one-way.
                return { state, events: [] };
            }
            return {
                state: { ...state, initPhase: { kind: "InitReady", at: command.at } },
                events: [{ type: "init-ready" }],
            };
        }

        case "InitFailed": {
            if (state.initPhase.kind !== "InitPending") {
                // Same one-way rule as InitReady — once terminal, stay.
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    initPhase: { kind: "InitFailed", at: command.at, reason: command.reason },
                },
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
            if (state.initPhase.kind === "InitPending") {
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
            // Codex P1 on #991: when an `InterruptTimeoutElapsed` has
            // already forced us to `Done.interrupted` (clearing `stopping`
            // in the process), a late backend `TurnEnd` ack would see
            // `stoppingWasSet === false` and classify the outcome as
            // "completed", overwriting the forced interrupted state.
            // First-done-wins: if we're already in `Done`, preserve the
            // existing outcome and only refresh the stats sidecar. The
            // late ack is confirmatory, not authoritative.
            const alreadyDone = state.turnPhase.kind === "Done";
            // Dual-write outcome: stop-in-flight → "stopped"; otherwise
            // "completed". "interrupted" / "errored" are reserved for
            // future-PR commands (StreamStalled, Disconnected, etc.).
            const outcome: TurnOutcome = alreadyDone
                ? state.turnPhase.outcome
                : stoppingWasSet
                    ? "stopped"
                    : "completed";
            const finishedAt = alreadyDone
                ? state.turnPhase.finishedAt
                : nowMs;
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
                        finishedAt,
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
            // PR C: only schedule the bounded-interrupt watchdog when we
            // actually CROSS INTO Interrupting (not when we're already
            // there — that would double-arm the timeout). Spec §8: SIGINT
            // is only emitted once on entry; the timeout follows the same
            // rule so a second Stop press doesn't reset the deadline.
            const enteringInterrupting =
                isWorking && k !== "Interrupting";
            // Codex P2 on #991: preserve the original sigintSentAt on a
            // repeated Stop press. The timeout was armed from the FIRST
            // press's deadline; rebuilding the phase with a fresh
            // `command.at` here makes consumers see a `sigintSentAt`
            // that doesn't match the active deadline. When we're
            // already Interrupting, reuse the existing phase so the
            // timestamp stays pinned to the original SIGINT.
            const nextPhase: TurnPhase = !isWorking
                ? state.turnPhase
                : k === "Interrupting"
                    ? state.turnPhase
                    : {
                          kind: "Interrupting",
                          reason: "user",
                          sigintSentAt: command.at,
                      };
            const events: AgentPaneEvent[] = [
                { type: "stop-requested", at: command.at },
            ];
            if (enteringInterrupting) {
                events.push({
                    type: "schedule-interrupt-timeout",
                    deadlineMs: command.at + INTERRUPT_TIMEOUT_MS,
                });
            }
            return {
                state: { ...state, stopping: true, turnPhase: nextPhase },
                events,
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

        case "InterruptTimeoutElapsed": {
            // PR C — bounded `Interrupting → Done.interrupted`.
            //
            // The dispatch side schedules a setTimeout when it sees the
            // `schedule-interrupt-timeout` event. A shared cancel flag
            // (the JS analogue of `Arc<AtomicBool>`) prevents a stale
            // timer from firing after a graceful TurnEnd / unsubscribe
            // already moved us off Interrupting — but defence in depth:
            // the reducer ALSO checks the current phase and treats a
            // late tick as a no-op. Three independent paths into
            // Done.interrupted (TurnEnd, StreamUnsubscribe → Disconnected,
            // and this timeout) — so the working animation can never
            // hang on Interrupting indefinitely. Spec §8.
            if (state.turnPhase.kind !== "Interrupting") {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    // Legacy: a forced exit from Interrupting is the
                    // same shape as a graceful TurnEnd for the boolean
                    // consumers (turnActive=false, stopping cleared).
                    turnActive: false,
                    stopping: false,
                    currentTool: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome: "interrupted",
                        finishedAt: command.at,
                    },
                },
                events: [{ type: "interrupt-timed-out", at: command.at }],
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
