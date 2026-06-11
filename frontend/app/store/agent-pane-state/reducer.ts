// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's lifecycle/turn/tool/tokens/
 * pending state. See docs/specs/agent-pane-document-reducer-2026-05-03.md
 * (slice #1 — pattern reference) and frontend-reducer-conventions-2026-05-03.md.
 *
 * Invariants enforced:
 *   1. A turn cannot enter Submitting (TurnStart) while the stream is
 *      unsubscribed (suppressed).
 *   2. A turn cannot enter Submitting while initPhase.kind ===
 *      "InitPending" (suppressed). After `InitFailed` we fail open —
 *      see the `TurnStart` arm.
 *   3. currentTool and turnTokens clear on TurnEnd / TurnReset.
 *   4. pending FIFO; accepting/rejecting/expiring unknown ids is no-op.
 *   5. StreamFlushObserved is a no-op when the stream is unsubscribed.
 *   6. StreamWatchdogTick is a no-op when the stream is unsubscribed or
 *      lastEventMs is null (no events seen yet).
 *   7. Init lifecycle transitions are one-way: InitStart / InitReady /
 *      InitFailed are no-ops once the pane has reached a terminal init
 *      state (InitReady / InitFailed). Re-entering pending requires a
 *      fresh pane slot.
 *
 * Stream-subscription gate: since PR G the legacy `streaming.active`
 * boolean is gone. `state.lastEventMs !== null` is the canonical "is
 * the stream subscribed?" check — both fields always moved in lockstep
 * (only StreamSubscribe / StreamUnsubscribe ever touched either) so
 * the substitution is mechanical. The {@link isStreamSubscribed} selector
 * wraps the check.
 */

import {
    AgentPaneCommand,
    AgentPaneEvent,
    AgentPaneState,
    INTERRUPT_TIMEOUT_MS,
    KindBeforeDisconnect,
    ReducerResult,
    STREAMING_IDLE_TIMEOUT_MS,
    STUCK_THRESHOLD_MS,
    SUBMIT_TIMEOUT_MS,
    TurnOutcome,
    TurnPhase,
} from "./types";
import type { DisconnectReason } from "./types";

/**
 * If `phase` is `Streaming`, return the `schedule-stream-watchdog` event
 * that re-arms the bounded idle-timeout against the latest event time.
 * The dispatcher cancels any prior arm and reschedules — so this is
 * called on EVERY stream-activity command that lands inside Streaming.
 * Spec §10 / PR E.
 */
function streamWatchdogEvent(
    phase: TurnPhase,
): AgentPaneEvent | null {
    if (phase.kind !== "Streaming") return null;
    return {
        type: "schedule-stream-watchdog",
        deadlineMs: phase.lastEventMs + STREAMING_IDLE_TIMEOUT_MS,
    };
}

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
            // its expected hand-off to Streaming. If already Streaming,
            // the resubscribe IS fresh activity — refresh
            // `lastEventMs` to `command.at` so the re-armed watchdog
            // deadline is in the future. (P1 on #995 from reagent +
            // codex: stale `lastEventMs` could schedule a deadline in
            // the past, firing `StreamStalled` immediately on a freshly-
            // resubscribed turn.)
            //
            // PR F: a subscribe from `Disconnected` clears the
            // disconnect — we land in `Idle` (not back in the working
            // set; the lost turn is gone, the next `TurnStart` promotes
            // to Submitting). Done stays Done (terminal). Idle stays
            // Idle (no-op).
            const nextPhase: TurnPhase =
                state.turnPhase.kind === "Submitting"
                    ? {
                          kind: "Streaming",
                          bufferSize: state.streaming.bufferSize,
                          toolsActive: 0,
                          lastEventMs: command.at,
                      }
                    : state.turnPhase.kind === "Streaming"
                        ? {
                              ...state.turnPhase,
                              lastEventMs: command.at,
                          }
                        : state.turnPhase.kind === "Disconnected"
                            ? { kind: "Idle" }
                            : state.turnPhase;
            const events: AgentPaneEvent[] = [
                { type: "stream-subscribed", at: command.at },
            ];
            // PR E: re-arm the bounded streaming watchdog on every
            // activity that lands inside Streaming. The refresh of
            // `lastEventMs` above ensures the new deadline is always
            // `command.at + STREAMING_IDLE_TIMEOUT_MS`, never in the past.
            const wd = streamWatchdogEvent(nextPhase);
            if (wd) events.push(wd);
            return {
                state: {
                    ...state,
                    streaming: {
                        ...state.streaming,
                        lastEventTime: command.at,
                    },
                    lastEventMs: command.at,
                    turnPhase: nextPhase,
                },
                events,
            };
        }

        case "StreamUnsubscribe": {
            // If we were working (Submitting/Streaming/Interrupting),
            // surface a Disconnected phase so the view can show
            // re-attach UX. Otherwise (Idle / Done / already
            // Disconnected) the unsubscribe is idempotent — return the
            // same state ref so reactive consumers don't see a phantom
            // tick. Spec §6.4 / PR F.
            const k = state.turnPhase.kind;
            const wasWorking =
                k === "Submitting" || k === "Streaming" || k === "Interrupting";
            if (!wasWorking) {
                // Same-ref no-op for Idle / Done / Disconnected. There
                // is no in-flight turn so per-turn sidecars are already
                // cleared. No event either — the unsubscribe is non-news.
                return { state, events: [] };
            }
            const reason: DisconnectReason = "stream-unsubscribed";
            const lastKind = k as KindBeforeDisconnect;
            const nextPhase: TurnPhase = {
                kind: "Disconnected",
                lastKind,
                lastConnectedAt: command.at,
                reason,
            };
            return {
                state: {
                    ...state,
                    // Per-turn sidecar cleanup mirrors the bounded
                    // force-transition arms (InterruptTimeoutElapsed /
                    // SubmitTimeoutElapsed / StreamStalled): a forced
                    // exit from a working state settles the animation
                    // and clears tool / token chips so the header
                    // doesn't show ghosts.
                    currentTool: null,
                    turnTokens: null,
                    lastEventMs: null,
                    turnPhase: nextPhase,
                },
                events: [
                    { type: "stream-unsubscribed", at: command.at },
                    {
                        type: "stream-disconnected",
                        at: command.at,
                        lastKind,
                        reason,
                    },
                ],
            };
        }

        case "StreamFlushObserved": {
            // Gate: only mutate if the stream is currently subscribed.
            // PR G — previously `state.streaming.active`; the
            // `lastEventMs !== null` check is exactly equivalent
            // (both moved in lockstep on subscribe/unsubscribe).
            if (state.lastEventMs == null) {
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
            const events: AgentPaneEvent[] = [
                { type: "stream-flush-observed", addedCount: command.addedCount },
            ];
            // PR E: flushes are the primary stream-activity signal; if
            // we landed inside Streaming, re-arm the idle watchdog.
            const wd = streamWatchdogEvent(nextPhase);
            if (wd) events.push(wd);
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
                events,
            };
        }

        case "StreamWatchdogTick": {
            // No-op when stream unsubscribed (lastEventMs null also
            // covers "no event seen yet" — same field since PR G
            // dropped the redundant `streaming.active` boolean).
            if (state.lastEventMs == null) {
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
            // Invariant 1: can't start a turn without a subscribed
            // stream. PR G: `state.lastEventMs !== null` replaces the
            // dropped `state.streaming.active` boolean — both moved
            // in lockstep so the gate is unchanged.
            if (state.lastEventMs == null) {
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
            // PR D: only schedule the submit watchdog when we actually
            // CROSS INTO Submitting (not when we're already there — that
            // would double-arm the timeout). Mirrors the PR C entry
            // pattern for the interrupt watchdog. In practice TurnStart
            // is dispatched once per user-send so we expect this branch
            // ~always, but defence in depth keeps the contract clean for
            // any future re-entrant caller.
            const enteringSubmitting = state.turnPhase.kind !== "Submitting";
            const events: AgentPaneEvent[] = [
                { type: "turn-started", at: command.at },
            ];
            if (enteringSubmitting) {
                events.push({
                    type: "schedule-submit-timeout",
                    deadlineMs: command.at + SUBMIT_TIMEOUT_MS,
                });
            }
            return {
                state: {
                    ...state,
                    sessionStats: null, // clear stale stats from prior turn
                    lastEventMs: command.at,
                    // Submitting until the first stream event /
                    // subscribe transitions us to Streaming. The
                    // TurnStart payload doesn't carry pendingContent;
                    // a later PR can thread that through.
                    turnPhase: {
                        kind: "Submitting",
                        submittedAt: command.at,
                        pendingContent: "",
                    },
                    // Auto-collapse the composer details panel on send:
                    // the user pressed Enter, they don't want to be
                    // looking at a dropdown over the textarea while the
                    // turn starts.
                    // SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4.
                    detailsOpen: false,
                },
                events,
            };
        }

        case "TurnEnd": {
            // PR G: replaces the legacy `state.stopping` boolean. A
            // stop-in-flight is exactly `turnPhase.kind === "Interrupting"`
            // since RequestStop is the only command that enters
            // Interrupting and the same arms that previously cleared
            // `stopping` are the ones that leave Interrupting.
            const stoppingWasSet = state.turnPhase.kind === "Interrupting";
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
            //
            // PR F extends the guard to `Disconnected` as well — Option A
            // from the PR F spec: a late TurnEnd that arrives after the
            // stream has already torn down (e.g. backend pushed a final
            // session_end into a buffer right before the PTY died, then
            // the buffered ack drains) must NOT overwrite the disconnect
            // with a synthetic Done. The disconnect is the authoritative
            // outcome; the late ack is informational. Same-ref no-op
            // preserves both the phase and the cleared legacy fields.
            if (state.turnPhase.kind === "Disconnected") {
                return { state, events: [] };
            }
            // Dual-write outcome: stop-in-flight → "stopped"; otherwise
            // "completed". "interrupted" / "errored" are reserved for
            // bounded force-transition arms (InterruptTimeoutElapsed,
            // SubmitTimeoutElapsed, StreamStalled).
            //
            // TS narrowing: read the phase via a local so the
            // `.kind === "Done"` ternary below narrows `outcome` /
            // `finishedAt` accesses (a `const alreadyDone = …` boolean
            // wouldn't carry the narrow into the ternary branches).
            const phase = state.turnPhase;
            const outcome: TurnOutcome = phase.kind === "Done"
                ? phase.outcome
                : stoppingWasSet
                    ? "stopped"
                    : "completed";
            const finishedAt = phase.kind === "Done"
                ? phase.finishedAt
                : nowMs;
            return {
                state: {
                    ...state,
                    sessionStats: merged,
                    currentTool: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome,
                        finishedAt,
                    },
                },
                events: [
                    {
                        type: "turn-ended",
                        outcome,
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
                    // TurnReset is a wholesale clear → Idle. The
                    // working/stopping cascade lives entirely on
                    // turnPhase since PR G.
                    turnPhase: { kind: "Idle" },
                },
                events: [{ type: "turn-reset" }],
            };

        case "ToolStart": {
            const nextState = bumpEvent(
                { ...state, currentTool: command.name },
                nowMs,
                /* toolsDelta */ +1,
            );
            const events: AgentPaneEvent[] = [
                { type: "tool-started", name: command.name },
            ];
            // PR E: re-arm the streaming watchdog when activity lands
            // inside Streaming (covers both promote-from-Submitting and
            // tools-active bump within Streaming).
            const wd = streamWatchdogEvent(nextState.turnPhase);
            if (wd) events.push(wd);
            return { state: nextState, events };
        }

        case "ToolEnd": {
            const nextState = bumpEvent(
                { ...state, currentTool: null },
                nowMs,
                /* toolsDelta */ -1,
            );
            const events: AgentPaneEvent[] = [{ type: "tool-ended" }];
            const wd = streamWatchdogEvent(nextState.turnPhase);
            if (wd) events.push(wd);
            return { state: nextState, events };
        }

        case "TokensIn": {
            const next = {
                input: command.input,
                output: state.turnTokens?.output ?? 0,
            };
            const nextState = bumpEvent(
                { ...state, turnTokens: next },
                nowMs,
                0,
            );
            const events: AgentPaneEvent[] = [
                { type: "tokens-updated", input: command.input, output: null },
            ];
            const wd = streamWatchdogEvent(nextState.turnPhase);
            if (wd) events.push(wd);
            return { state: nextState, events };
        }

        case "TokensOut": {
            const next = {
                input: state.turnTokens?.input ?? 0,
                output: command.output,
            };
            const nextState = bumpEvent(
                { ...state, turnTokens: next },
                nowMs,
                0,
            );
            const events: AgentPaneEvent[] = [
                { type: "tokens-updated", input: null, output: command.output },
            ];
            const wd = streamWatchdogEvent(nextState.turnPhase);
            if (wd) events.push(wd);
            return { state: nextState, events };
        }

        case "RequestStop": {
            // If a turn is in flight, surface Interrupting; otherwise
            // the stop is a no-op (no working state to interrupt).
            // PR G removed the legacy `stopping` boolean — there is no
            // hidden "stop pending" mode outside the working set.
            const k = state.turnPhase.kind;
            const isWorking =
                k === "Submitting" || k === "Streaming" || k === "Interrupting";
            if (!isWorking) {
                // No-op stop — emit `stop-requested` for diagnostic
                // continuity but leave state untouched. The legacy
                // boolean used to flip true here even with no turn in
                // flight; that was a latent bug surface (the stop
                // could not actually be acted on) and PR G drops it.
                return {
                    state,
                    events: [{ type: "stop-requested", at: command.at }],
                };
            }
            // PR C: only schedule the bounded-interrupt watchdog when we
            // actually CROSS INTO Interrupting (not when we're already
            // there — that would double-arm the timeout). Spec §8: SIGINT
            // is only emitted once on entry; the timeout follows the same
            // rule so a second Stop press doesn't reset the deadline.
            const enteringInterrupting = k !== "Interrupting";
            // Codex P2 on #991: preserve the original sigintSentAt on a
            // repeated Stop press. The timeout was armed from the FIRST
            // press's deadline; rebuilding the phase with a fresh
            // `command.at` here makes consumers see a `sigintSentAt`
            // that doesn't match the active deadline. When we're
            // already Interrupting, reuse the existing phase so the
            // timestamp stays pinned to the original SIGINT.
            const nextPhase: TurnPhase =
                k === "Interrupting"
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
                state: { ...state, turnPhase: nextPhase },
                events,
            };
        }

        case "StopFailed": {
            // Rollback Interrupting → previous working kind best-effort.
            // We don't track the prior kind on Interrupting, so use
            // the stream-subscribed gate as the deterministic signal:
            //   subscribed (lastEventMs !== null) → Streaming (common case)
            //   else                              → Idle (no longer in a turn)
            // Non-Interrupting phases are a no-op — there's nothing
            // to roll back. PR G dropped the legacy `stopping` boolean
            // that previously also cleared here.
            const k = state.turnPhase.kind;
            const nextPhase: TurnPhase =
                k === "Interrupting"
                    ? state.lastEventMs !== null
                        ? {
                              kind: "Streaming",
                              bufferSize: state.streaming.bufferSize,
                              toolsActive: 0,
                              lastEventMs: state.lastEventMs,
                          }
                        : { kind: "Idle" }
                    : state.turnPhase;
            return {
                state: { ...state, turnPhase: nextPhase },
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
                    // Per-turn sidecars cleared the same way TurnEnd
                    // does — the working spinner settles and the
                    // header doesn't show ghost tool/token chips.
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

        case "SubmitTimeoutElapsed": {
            // PR D — bounded `Submitting → Done.errored`.
            //
            // The dispatch side schedules a setTimeout when it sees the
            // `schedule-submit-timeout` event emitted by `TurnStart`. A
            // shared cancel flag (the JS analogue of `Arc<AtomicBool>`,
            // same pattern as the PR C interrupt timer) prevents a stale
            // timer from firing after a graceful promotion to Streaming
            // (via `StreamFlushObserved` or `bumpEvent` from a tool /
            // token event). Defence in depth: the reducer ALSO checks
            // the current phase and treats a late tick as a no-op.
            //
            // Three independent paths out of Submitting — promotion via
            // `StreamFlushObserved`, promotion via `bumpEvent` (tool /
            // token), and this timeout — so the working animation can
            // never get stuck on Submitting indefinitely. The first
            // promotion wins; this timeout fires only if none arrived.
            // Spec §8 / issue #728 gap 2.
            if (state.turnPhase.kind !== "Submitting") {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    // Per-turn sidecar cleanup mirrors the
                    // InterruptTimeoutElapsed arm — a forced exit from
                    // a working state clears tool / token chips.
                    currentTool: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome: "errored",
                        finishedAt: command.at,
                    },
                },
                events: [{ type: "submit-timed-out", at: command.at }],
            };
        }

        case "StreamStalled": {
            // PR E — bounded `Streaming → Done.errored("stream-stalled")`.
            //
            // The dispatcher schedules a setTimeout when it sees the
            // `schedule-stream-watchdog` event; on every fresh stream
            // event the reducer re-emits the schedule and the
            // dispatcher cancels the previous timer (shared cancel
            // flag — JS analogue of `Arc<AtomicBool>`). When the timer
            // finally fires, the reducer makes the final decision:
            //
            //   - `Streaming` + idle ≥ `STREAMING_IDLE_TIMEOUT_MS` →
            //     force-transition to `Done.errored`. Legacy fields are
            //     cleared the same way TurnEnd / interrupt-timeout do.
            //   - `Streaming` + NOT yet stale (early callback or a
            //     just-arrived event already refreshed `lastEventMs`)
            //     → same-ref no-op. The dispatcher re-arms on the next
            //     activity command — defence in depth against a leaked
            //     timer firing right after an event landed.
            //   - any other phase → same-ref no-op. The timer is a
            //     stale leftover from a prior turn; harmless.
            //
            // First-done-wins: if a graceful `TurnEnd` already landed
            // us in `Done`, the late stalled tick can't reach this arm
            // (the phase check is `=== "Streaming"`), so the outcome
            // is preserved. Mirrors the `Interrupting → Done` timeout
            // pattern from PR C. Spec §10.
            if (state.turnPhase.kind !== "Streaming") {
                return { state, events: [] };
            }
            const idleMs = command.at - state.turnPhase.lastEventMs;
            if (idleMs < STREAMING_IDLE_TIMEOUT_MS) {
                // Early callback / race: real activity refreshed
                // `lastEventMs` between the dispatcher seeing the timer
                // pop and the command landing. No-op; the next activity
                // command will re-arm.
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    // Per-turn sidecar cleanup mirrors TurnEnd —
                    // animation settles, currentTool/turnTokens cleared
                    // so the header doesn't show ghost tool/token chips.
                    currentTool: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome: "errored",
                        finishedAt: command.at,
                    },
                },
                events: [{ type: "stream-stalled", at: command.at }],
            };
        }

        case "PendingMessageQueued":
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
                        },
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

        // ── Composer details panel ─────────────────────────────────
        // The chevron/strip toggle, explicit expand, explicit collapse,
        // and activity-log unread-counter arms. The view dispatches all
        // four; sagas are uninterested (no scheduled side effects).
        // Spec: SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4.
        case "DetailsToggle": {
            const opening = !state.detailsOpen;
            return {
                state: {
                    ...state,
                    detailsOpen: opening,
                    // On flip-to-open, reset the unread counter — the
                    // entries are now visible. On flip-to-close, leave
                    // the counter alone (it will accumulate fresh).
                    composerUnreadCount: opening ? 0 : state.composerUnreadCount,
                },
                events: [],
            };
        }
        case "DetailsExpand": {
            // Idempotent if already open (same-ref no-op preserves
            // reactive identity).
            if (state.detailsOpen && state.composerUnreadCount === 0) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    detailsOpen: true,
                    composerUnreadCount: 0,
                },
                events: [],
            };
        }
        case "DetailsCollapse": {
            // Idempotent if already closed. Unread counter is NOT reset
            // — it tracks entries arriving while collapsed, so a re-open
            // a moment later should see only what arrived in between.
            if (!state.detailsOpen) {
                return { state, events: [] };
            }
            return {
                state: { ...state, detailsOpen: false },
                events: [],
            };
        }
        case "LogEntryArrived": {
            // No-op when the panel is open (user already sees the
            // entry). Increments the unread counter when closed so
            // the chevron badge updates.
            if (state.detailsOpen) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    composerUnreadCount: state.composerUnreadCount + 1,
                },
                events: [],
            };
        }
    }
}

/**
 * Bump `lastEventMs` to "now" only while the stream is subscribed. Used
 * by tool/token transitions which are observable stream activity. Skip
 * the bump when the stream is unsubscribed — those would correspond to
 * stale commands and shouldn't reset the watchdog clock.
 *
 * PR G: the subscribed gate is now `state.lastEventMs !== null` (was
 * `state.streaming.active`; they always moved in lockstep). If the
 * phase is Streaming, also update its `lastEventMs` and apply
 * `toolsDelta` to `toolsActive` (clamped ≥ 0).
 */
function bumpEvent(
    state: AgentPaneState,
    nowMs: number,
    toolsDelta: number,
): AgentPaneState {
    if (state.lastEventMs == null) return state;
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
 * Merge the turn's token totals into sessionStats. Prefer the result
 * event's whole-turn totals over the live turn-tokens, which hold only
 * the last message_start/message_delta (TokensIn/TokensOut overwrite),
 * fall back to the live turn-tokens only when the result reported no/zero
 * usage (hence `||`, not `??`). Codex emits usage only on the stats
 * branch (no live tokens), so it's unaffected.
 */
function mergeStats(
    stats: AgentPaneState["sessionStats"],
    tokens: AgentPaneState["turnTokens"],
): AgentPaneState["sessionStats"] {
    if (stats) {
        return {
            ...stats,
            input_tokens: stats.input_tokens || tokens?.input,
            output_tokens: stats.output_tokens || tokens?.output,
        };
    }
    if (tokens) {
        return { input_tokens: tokens.input, output_tokens: tokens.output };
    }
    return null;
}
