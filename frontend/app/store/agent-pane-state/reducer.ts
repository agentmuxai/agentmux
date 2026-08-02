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
 *      lastEventMs is null (no events seen yet); past LIVENESS_RECOVERY_MS it
 *      force-recovers a hung Streaming turn (no tool active) to Idle — and for a
 *      stalled rate-limit wait, past retryAfterMs + LIVENESS_RECOVERY_MS.
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
    COMPACTION_HEURISTIC_SUPPRESS_MS,
    INTERRUPT_TIMEOUT_MS,
    KindBeforeDisconnect,
    LIVENESS_RECOVERY_MS,
    ReducerResult,
    STUCK_THRESHOLD_MS,
    SUBMIT_TIMEOUT_MS,
    TurnOutcome,
    TurnPhase,
    workingFromPhase,
} from "./types";
import type { DisconnectReason } from "./types";
import { learnContextWindow } from "./context-window";

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
            // `lastEventMs` to `command.at` so the stuck-stream
            // diagnostic (`StreamWatchdogTick` → `stream-stuck`) measures
            // idle time from the resubscribe, not from a stale prior
            // event. (P1 on #995 from reagent + codex.)
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
                    // SubmitTimeoutElapsed): a forced
                    // exit from a working state settles the animation
                    // and clears tool / token chips so the header
                    // doesn't show ghosts.
                    currentTool: null,
                    currentToolArg: null,
                    turnTokens: null,
                    lastEventMs: null,
                    turnPhase: nextPhase,
                    // reagent P1 on PR #2378: a disconnect mid-compaction
                    // (crash, network drop, reconnect race) means the
                    // matching CompactionBoundary will never arrive to
                    // clear this — without this, the composer strip is
                    // stuck showing "Compacting… Ns" forever, surviving
                    // the reconnect and every subsequent turn.
                    compacting: null,
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
            // PROMOTE to Streaming from Submitting (the normal hand-off) AND
            // from Idle / Disconnected. `StreamFlushObserved` is only dispatched
            // for LIVE stream content (never history replay), so a flush is
            // proof the agent is producing output — it must re-enter the working
            // set. After a stream drop + resubscribe (e.g. an agent kill+respawn
            // during a long upstream stall) the phase lands in Idle/Disconnected,
            // and without this promotion the "in progress" indicator stays OFF
            // while output streams in. Done.completed is included: session_end
            // fires after every model API round, so Done.completed can mean
            // "first round of a multi-round tool-continuation finished" — live
            // flush content arriving in that state is proof the agent picked up
            // another round. Done.errored / Done.stopped / Done.interrupted are
            // excluded: a submit-timeout or user-stop followed by a late stray
            // flush must not re-activate the busy indicator on a failed turn.
            // Interrupting (user is stopping) is also intentionally excluded.
            const nextPhase: TurnPhase =
                state.turnPhase.kind === "Streaming"
                    ? {
                          ...state.turnPhase,
                          bufferSize: newBuf,
                          lastEventMs: command.at,
                          // Clear a stale rate-limit wait on ANY observable
                          // stream activity, not just tool/token events —
                          // `bumpEvent` already does this; this branch
                          // previously spread the whole phase forward
                          // unchanged, so a rate-limited turn whose very
                          // next activity was plain streamed text (no
                          // intervening tool call) kept showing "Rate
                          // limited — retrying…" long after the agent had
                          // resumed normal streaming (reported false
                          // positive, see
                          // ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md).
                          waitingReason: undefined,
                          retryAfterMs: undefined,
                      }
                    : state.turnPhase.kind === "Submitting"
                        || state.turnPhase.kind === "Idle"
                        || state.turnPhase.kind === "Disconnected"
                        || (state.turnPhase.kind === "Done" && state.turnPhase.outcome === "completed")
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
            // Codex P1 on PR #2378 (round 2): CompactionStarted only bumps
            // lastEventMs ONCE, at the moment compaction begins — it isn't
            // re-bumped on every tick while compaction is still running.
            // The captured real example (spec doc §2) took ~232s: past
            // STUCK_THRESHOLD_MS (45s) alone that's a cosmetic false
            // "stream-stuck" warning, but past LIVENESS_RECOVERY_MS (180s)
            // it would force-demote a perfectly healthy, actively-
            // compacting Streaming turn to Idle out from under it. Suspend
            // the watchdog entirely for as long as `compacting` is set —
            // CompactionBoundary (or any of the other lifecycle events that
            // clear `compacting`, see the tests above) re-arms it.
            if (state.compacting != null) {
                return { state, events: [] };
            }
            const idleSinceMs = command.nowMs - state.lastEventMs;
            if (idleSinceMs < STUCK_THRESHOLD_MS) {
                return { state, events: [] };
            }
            // Liveness recovery (SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29).
            // A `Streaming` turn that goes idle too long with no tool running is
            // hung: its terminal `session_end` was never observed. For persistent
            // agents no `ControllerStatus: done` will ever arrive to clear it (the
            // process doesn't exit between turns), so the watchdog itself
            // transitions out of "Working". Land in `Idle` (turn abandoned) — NOT
            // a synthetic `Done.completed`, which would feed false stats to session
            // digests. If real activity arrives after recovery, `bumpEvent`
            // re-promotes Idle → Streaming, so a late recovery self-corrects.
            //
            // Threshold depends on the wait state:
            //  - normal stall → LIVENESS_RECOVERY_MS (a running tool keeps
            //    `lastEventMs` fresh, so an active tool never reaches it).
            //  - rate-limit stall → retryAfterMs + LIVENESS_RECOVERY_MS. A genuine
            //    429 backoff re-emits `provider_waiting` within `retryAfterMs`
            //    (refreshing `lastEventMs`), so it only accumulates this much idle
            //    if the retry loop itself stalled — the Claude CLI emitted one
            //    `rate_limit_event` then went silent with no follow-up event,
            //    `session_end`, or process exit. Waiting out the advertised retry
            //    window PLUS the liveness window guarantees a long-but-live backoff
            //    is never mistaken for a hang. (Pre-fix, rate-limited turns were
            //    excluded from recovery entirely — see
            //    docs/retro/retro-busy-animation-stuck-on-429-2026-06-24.md.)
            // Interrupting/Submitting are left to their own bounded timers
            // (INTERRUPT_TIMEOUT_MS / SUBMIT_TIMEOUT_MS).
            const phase = state.turnPhase;
            if (phase.kind === "Streaming" && phase.toolsActive === 0) {
                const recoverThresholdMs =
                    phase.waitingReason === "rate_limited"
                        ? (phase.retryAfterMs ?? 0) + LIVENESS_RECOVERY_MS
                        : LIVENESS_RECOVERY_MS;
                if (idleSinceMs >= recoverThresholdMs) {
                    return {
                        state: {
                            ...state,
                            currentTool: null,
                            currentToolArg: null,
                            turnTokens: null,
                            turnPhase: { kind: "Idle" },
                        },
                        events: [
                            {
                                type: "working-recovered",
                                idleSinceMs,
                                thresholdMs: recoverThresholdMs,
                            },
                        ],
                    };
                }
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

        case "ReconcileTurnActive": {
            if (command.active) {
                // Promote from the mount-default Idle (this seed's original
                // purpose — covers the window before the live stream has
                // necessarily subscribed yet, see the type's doc comment in
                // types.ts) OR from a settled Done.completed episode. The
                // Done.completed case matters for the focus-triggered
                // reconcile (agent-view.tsx): a pane can be showing "Worked"
                // while backgrounded, a genuinely new turn starts, AND the
                // live turn-start signal is also missed — without this, the
                // authoritative "yes, a turn really is active" RPC response
                // this command carries would silently no-op, leaving the
                // pane stuck on "Worked" despite real work in progress
                // (reagent P1 on the PR that added the focus reconcile).
                // Applies the SAME standard StreamFlushObserved already uses
                // for live stream content proving a continuation — this
                // authoritative snapshot is an equally strong (arguably
                // stronger) signal of truth. Other Done outcomes (stopped/
                // errored/interrupted) stay excluded, matching
                // StreamFlushObserved: a failed/stopped turn must not be
                // silently re-activated. Never overrides Streaming/
                // Submitting/Interrupting/Disconnected — those already
                // reflect a real stream/user event this snapshot must not
                // clobber. Deliberately does NOT gate on `state.lastEventMs`
                // the way TurnStart / StreamFlushObserved do, for the same
                // reason the original Idle case didn't.
                const canPromote =
                    state.turnPhase.kind === "Idle" ||
                    (state.turnPhase.kind === "Done" && state.turnPhase.outcome === "completed");
                if (!canPromote) {
                    return { state, events: [] };
                }
                return {
                    state: {
                        ...state,
                        turnPhase: {
                            kind: "Streaming",
                            bufferSize: 0,
                            toolsActive: 0,
                            lastEventMs: command.at,
                        },
                    },
                    events: [{ type: "turn-active-reconciled" }],
                };
            }
            // active === false: the backend's authoritative turn_active (flipped
            // false only on the CLI's `result` event = genuine turn-end) says
            // the turn is over. Demote a STUCK Streaming phase to Idle. The
            // frontend normally reaches Done itself via session_end; this only
            // matters when it missed that event and would stay Streaming forever
            // (Agent1 stuck-"Working" / Agent2 stuck-"Queued" — the latter's
            // held-message flush is gated on the phase reaching Idle/Done, so a
            // stuck Streaming strands the queue). ONLY Streaming: Submitting is
            // left to SUBMIT_TIMEOUT (and is where the send-race lives — a local
            // TurnStart racing a backend that hasn't marked the turn active yet,
            // which would arrive here as a stale active:false), Interrupting to
            // INTERRUPT_TIMEOUT, and Done/Idle/Disconnected are already correct.
            // Clears currentTool/turnTokens exactly like the liveness watchdog's
            // recovery, but on an authoritative signal rather than a timeout.
            if (state.turnPhase.kind !== "Streaming") {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    currentTool: null,
                    currentToolArg: null,
                    turnTokens: null,
                    turnPhase: { kind: "Idle" },
                    // Same reasoning as the other authoritative terminal
                    // transitions above: the backend has just confirmed
                    // there is no active turn, so whatever compaction the
                    // frontend still thought was in flight is stale.
                    compacting: null,
                },
                events: [{ type: "turn-inactive-reconciled", at: command.at }],
            };
        }

        case "ReconcileContextFromHistory": {
            // Only ever seeds the mount-default null — never overrides a
            // real live TokensIn (or an earlier reconciliation) that already
            // landed. Mirrors ReconcileTurnActive's only-if-still-default
            // guard above.
            if (state.lastContextTokens != null) {
                return { state, events: [] };
            }
            return {
                state: { ...state, lastContextTokens: command.tokens },
                events: [{ type: "context-reconciled-at-mount", tokens: command.tokens }],
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
            // A fresh turn starting always ends any prior failure episode —
            // this is what the failure-recovery hook used to reconstruct by
            // watching raw ControllerStatus transitions (`awaitingVerdict`);
            // TurnStart already knows "a new turn is beginning" directly, so
            // there's no need to re-derive it from the outside. Emit
            // failure-cleared only when there was actually something to
            // clear, matching the other no-op-vs-event conventions in this
            // reducer (e.g. DetailsExpand/DetailsCollapse below).
            if (state.failure) {
                events.push({ type: "failure-cleared" });
            }
            return {
                state: {
                    ...state,
                    sessionStats: null, // clear stale stats from prior turn
                    lastEventMs: command.at,
                    failure: null,
                    // Submitting until the first stream event /
                    // subscribe transitions us to Streaming. The
                    // TurnStart payload doesn't carry pendingContent;
                    // a later PR can thread that through.
                    turnPhase: {
                        kind: "Submitting",
                        submittedAt: command.at,
                        pendingContent: "",
                    },
                    // Previously auto-collapsed the details panel on send (don't
                    // obscure the turn start). The panel now also hosts a live
                    // interactive shell (AgentShellSubblock) whose whole point is
                    // surviving across turns, so force-closing it on every message
                    // would fight that — leave detailsOpen as the user set it.
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
            // SubmitTimeoutElapsed).
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
                    sessionTotals: accumulateStats(state.sessionTotals, merged),
                    currentTool: null,
                    currentToolArg: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome,
                        finishedAt,
                    },
                    // reagent P1 on PR #2378: the turn ending (however it
                    // ended) means whatever compaction was in flight is
                    // moot — a CompactionBoundary for it, if it ever
                    // arrives, would be stale. See the same note on
                    // StreamUnsubscribe above.
                    compacting: null,
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
                    sessionTotals: null,
                    currentTool: null,
                    currentToolArg: null,
                    turnTokens: null,
                    lastContextTokens: 0,
                    // TurnReset is a wholesale clear → Idle. The
                    // working/stopping cascade lives entirely on
                    // turnPhase since PR G.
                    turnPhase: { kind: "Idle" },
                    // reagent P1 on PR #2378: same "wholesale clear"
                    // reasoning extends to `compacting` — a reset must
                    // not leave a stale "Compacting…" readout behind.
                    compacting: null,
                },
                events: [{ type: "turn-reset" }],
            };

        case "TurnStartFailed":
            // Deliberately touches ONLY turnPhase (+ compacting, see below)
            // — see this command's doc
            // comment in types.ts for why TurnReset's wholesale clear is
            // wrong here (a transient send failure must not wipe
            // sessionStats/sessionTotals/lastContextTokens accumulated by
            // prior real turns in this same pane).
            // `compacting` IS cleared, unlike the fields above: codex P2
            // on PR #2378 (round 8) — a `compaction_started` ping can
            // land while this (new, doomed) turn attempt is briefly
            // Submitting (round 5's workingFromPhase gate explicitly
            // permits Submitting), then this arm reverts to Idle without
            // ever getting a matching CompactionBoundary for a turn that
            // never really started — same bug class as
            // SubmitTimeoutElapsed/InterruptTimeoutElapsed/
            // ReconcileTurnActive above, missed here in round 4 on the
            // (wrong) assumption that a synchronously-failed turn-start
            // could never race a live WPS ping.
            return {
                state: { ...state, turnPhase: { kind: "Idle" }, compacting: null },
                events: [{ type: "turn-start-failed" }],
            };

        case "ToolStart": {
            const nextState = bumpEvent(
                { ...state, currentTool: command.name, currentToolArg: command.arg ?? null },
                nowMs,
                /* toolsDelta */ +1,
            );
            const events: AgentPaneEvent[] = [
                { type: "tool-started", name: command.name },
            ];
            return { state: nextState, events };
        }

        case "ToolEnd": {
            const nextState = bumpEvent(
                { ...state, currentTool: null, currentToolArg: null },
                nowMs,
                /* toolsDelta */ -1,
            );
            const events: AgentPaneEvent[] = [{ type: "tool-ended" }];
            return { state: nextState, events };
        }

        case "TokensIn": {
            const next = {
                input: command.input,
                output: state.turnTokens?.output ?? 0,
            };
            // Learn the context window from the resolved model + observed fill
            // (seed-then-high-water-upgrade); null until a recognised model is
            // seen, so the view falls back to the provider's static window.
            const learnedWindow =
                learnContextWindow(
                    state.lastContextWindow,
                    command.input,
                    command.model,
                    state.lastContextModel,
                ) ?? state.lastContextWindow;
            const nextState = bumpEvent(
                {
                    ...state,
                    turnTokens: next,
                    lastContextTokens: command.input,
                    lastContextWindow: learnedWindow ?? null,
                    lastContextModel: command.model ?? state.lastContextModel,
                },
                nowMs,
                0,
            );
            const events: AgentPaneEvent[] = [
                { type: "tokens-updated", input: command.input, output: null },
            ];
            // Detect context compaction: token count drops ≥50% from a
            // non-trivial baseline. Compaction typically drops 80–95%;
            // normal turn-to-turn growth is monotonically increasing.
            // AgentMux /clear is frontend-only and does not affect tokens.
            //
            // Backstop only for Claude: a REAL `CompactionBoundary` event
            // (exact backend data, not inferred) already fired its own
            // `context-compacted` and reconciled `lastContextTokens` to
            // `postTokens` — suppress this heuristic within the window
            // below so the same boundary doesn't produce two events.
            // Providers without a structured signal (codex/gemini/copilot)
            // never set `lastCompactionBoundaryAt`, so the heuristic stays
            // fully active for them. See
            // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
            const prev = state.lastContextTokens;
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
                // Codex P2 on PR #2378 (round 3): deliberately does NOT
                // clear `compacting` here (an earlier version of this fix
                // did, per a since-superseded reagent finding). RequestStop
                // only SENDS a SIGINT — it doesn't confirm the turn
                // actually ended. If the stop fails, `StopFailed` rolls
                // the phase back to Streaming with no way to know whether
                // compaction resumed unaffected the whole time (the SIGINT
                // never landed) — eagerly clearing here would have
                // silently dropped the "Compacting…" status and its timer
                // for a compaction that was never actually interrupted.
                // TurnEnd/TurnReset/StreamUnsubscribe/FailureObserved
                // already clear `compacting` on every path that actually,
                // authoritatively ends the turn (whether the stop
                // succeeded or the turn ended some other way) — that's
                // sufficient; this transition doesn't need its own clear.
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
                    currentTool: null,
                    currentToolArg: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome: "interrupted",
                        finishedAt: command.at,
                    },
                    // reagent + codex P1 on PR #2378 (round 4): a fifth
                    // authoritative terminal transition, same bug class as
                    // the four already fixed. Round 3 deliberately stopped
                    // RequestStop from clearing `compacting` (so it survives
                    // a failed stop attempt) — but if the interrupt instead
                    // TIMES OUT here, that IS an authoritative end of the
                    // turn, same as TurnEnd/TurnReset/StreamUnsubscribe/
                    // FailureObserved, and must clear it the same way.
                    compacting: null,
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
                    currentTool: null,
                    currentToolArg: null,
                    turnTokens: null,
                    turnPhase: {
                        kind: "Done",
                        outcome: "errored",
                        finishedAt: command.at,
                    },
                    // Same reasoning as InterruptTimeoutElapsed above:
                    // realistically compaction only happens once Streaming
                    // has started, so `compacting` shouldn't be set while
                    // still Submitting — but CompactionStarted's own
                    // handling doesn't gate on phase kind, so clear it here
                    // too for the same defensive completeness reagent/codex
                    // asked for on the other bounded-timeout arm.
                    compacting: null,
                },
                events: [{ type: "submit-timed-out", at: command.at }],
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

        // ── Composer details / Log panel ──────────────────────────
        case "ProviderWaiting": {
            if (state.lastEventMs == null) return { state, events: [] };
            const next: AgentPaneState = { ...state, lastEventMs: command.at };
            if (next.turnPhase.kind === "Streaming") {
                next.turnPhase = {
                    ...next.turnPhase,
                    lastEventMs: command.at,
                    waitingReason: command.reason,
                    retryAfterMs: command.retryAfterMs,
                };
            }
            return {
                state: next,
                events: [{ type: "provider-waiting", reason: command.reason }],
            };
        }

        case "FailureObserved": {
            // Unconditionally end a working turn: a backend failure
            // classification is authoritative regardless of whether the
            // underlying CLI process ever exits (persistent-mode agents
            // don't, between turns — see useAgentStream.ts's process-exit
            // grace timer, which only covers the crash-exit case). Without
            // this, a rate-limited (or otherwise failed) persistent-mode
            // turn left `turnPhase` stuck in `Streaming` until the ~3-minute
            // liveness-recovery watchdog eventually cleared it — the
            // "stuck Waiting after a rate-limit interruption" bug.
            const turnWasEnded = workingFromPhase(state.turnPhase);
            const nextPhase: TurnPhase = turnWasEnded
                ? { kind: "Done", outcome: "errored", finishedAt: command.at }
                : state.turnPhase; // already idle — a stray/late event; leave phase alone
            return {
                state: {
                    ...state,
                    turnPhase: nextPhase,
                    currentTool: turnWasEnded ? null : state.currentTool,
                    currentToolArg: turnWasEnded ? null : state.currentToolArg,
                    turnTokens: turnWasEnded ? null : state.turnTokens,
                    failure: { data: command.failure, at: command.at },
                    // reagent P1 on PR #2378 (round 3): a failure classification
                    // ends the turn the same way TurnEnd/TurnReset/RequestStop/
                    // StreamUnsubscribe do — same "whatever compaction was in
                    // flight is now moot" reasoning as those, just reached via
                    // an error instead of a clean transition. Without this, a
                    // failure observed mid-compaction (e.g. the CLI erroring out
                    // partway through) reproduces the exact stuck-"Compacting…"
                    // bug this PR already fixed for the other four transitions.
                    compacting: turnWasEnded ? null : state.compacting,
                },
                events: [
                    { type: "failure-observed", code: command.failure.code, turnWasEnded },
                ],
            };
        }

        case "FailureCleared": {
            if (!state.failure) return { state, events: [] };
            return {
                state: { ...state, failure: null },
                events: [{ type: "failure-cleared" }],
            };
        }

        case "CompactionStarted": {
            // Mirrors ProviderWaiting's "observable activity" bump — a
            // long compaction with no other stream output must not trip
            // the stuck-stream watchdog. No-op if the stream isn't
            // subscribed (a stray/late event after teardown).
            //
            // reagent P1 on PR #2378 (round 5): also no-op unless the turn
            // is actually in the working set (Submitting/Streaming/
            // Interrupting). `compaction_started` arrives over a SEPARATE
            // transport (WPS: HTTP publish -> broker -> websocket) from the
            // primary NDJSON stream carrying TurnEnd/compact_boundary, so
            // it can race and land AFTER that same turn's TurnEnd already
            // fired. Every "clear compacting" fix added across rounds 1-4
            // is itself gated on transitioning OUT of a working phase — if
            // `compacting` gets set while the pane is already Idle/Done,
            // none of those transitions ever fire again to clear it, and it
            // stays orphaned until the pane's next full TurnEnd/TurnReset —
            // the exact bug class this PR patched five separate times, just
            // reached from the setter side instead of a missing clearer.
            // Gating here is the structural fix: refuse to set stale-by-
            // construction state instead of enumerating every possible way
            // to clear it after the fact.
            //
            // reagent (round 6, PLAUSIBLE): the round-5 guard above doesn't
            // catch a NARROWER race — a stale `compaction_started` ping can
            // also arrive AFTER its own matching `CompactionBoundary` while
            // the turn is STILL working (e.g. streaming new content past
            // the compaction that already completed), since `workingFromPhase`
            // stays true the whole time and doesn't distinguish "before" from
            // "after" the boundary. `compaction_started` and `compact_boundary`
            // travel over two independent transports (WPS vs. the primary
            // NDJSON stream) with no ordering guarantee between them. Reject
            // any start whose own timestamp is at or before the most recent
            // known boundary — a genuinely NEW compaction must be later than
            // the previous one's own completion.
            const isStaleVsLastBoundary =
                state.lastCompactionBoundaryAt != null && command.at <= state.lastCompactionBoundaryAt;
            if (state.lastEventMs == null || !workingFromPhase(state.turnPhase) || isStaleVsLastBoundary) {
                return { state, events: [] };
            }
            const next: AgentPaneState = {
                ...state,
                lastEventMs: command.at,
                compacting: { trigger: command.trigger, startedAt: command.at },
            };
            if (next.turnPhase.kind === "Streaming") {
                next.turnPhase = { ...next.turnPhase, lastEventMs: command.at };
            }
            return {
                state: next,
                events: [{ type: "compaction-started", trigger: command.trigger }],
            };
        }

        case "CompactionBoundary": {
            // Codex P2 on PR #2378 (round 8): compact_boundary and
            // compaction_started travel over two independent transports
            // with no ordering guarantee (same root cause as the
            // CompactionStarted staleness guard above, mirrored here). If
            // compaction N+1 has already started (state.compacting) before
            // a DELAYED boundary for compaction N arrives, clearing
            // `compacting` unconditionally would wipe the genuinely
            // active N+1 state using stale N data. Only clear it when this
            // boundary's own completion time is at or after the
            // currently-tracked start — i.e. it's this same compaction (or
            // an even older one) finishing, not a stale echo racing a
            // newer one that's already begun. Falls back to clearing when
            // `frameTimestamp` is unavailable/unparseable — can't tell,
            // and a permanently-stuck "Compacting…" is worse than an
            // occasional early clear.
            const boundaryAt = command.frameTimestamp != null ? Date.parse(command.frameTimestamp) : NaN;
            const preservesNewerCompaction =
                state.compacting != null && !Number.isNaN(boundaryAt) && boundaryAt < state.compacting.startedAt;
            const next: AgentPaneState = {
                ...state,
                compacting: preservesNewerCompaction ? state.compacting : null,
                // Codex P2 on PR #2378 (round 10): use this boundary's own
                // parsed completion time, not `command.at` (the frontend's
                // receipt wall-clock). `CompactionStarted.at` is the WPS
                // payload's embedded true start time, not a receipt
                // timestamp — comparing it against a delayed boundary's
                // RECEIPT time in the isStaleVsLastBoundary check above
                // means network/stream lag on this (older) boundary's
                // delivery can exceed a legitimately-earlier next
                // compaction's true start, falsely rejecting it. Falls
                // back to `command.at` when frameTimestamp is unparseable,
                // same as preservesNewerCompaction above.
                lastCompactionBoundaryAt: Number.isNaN(boundaryAt) ? command.at : boundaryAt,
                // reagent P2 on PR #2378 (round 11): gated the same way as
                // `compacting` above. When this boundary belongs to an
                // OLDER compaction that's finishing after a newer one has
                // already started (preservesNewerCompaction), its
                // postTokens describes a context-fill state that's already
                // stale -- overwriting the live lastContextTokens with it
                // would show a smaller/incorrect reading while the newer
                // compaction is still confirmed in flight. The
                // context-compacted event below still reports this
                // boundary's own true tokens (accurate historical record
                // of what that specific compaction did); only the live
                // state gate changes here.
                lastContextTokens: preservesNewerCompaction ? state.lastContextTokens : command.postTokens,
            };
            if (state.lastEventMs != null) {
                next.lastEventMs = command.at;
                if (next.turnPhase.kind === "Streaming") {
                    next.turnPhase = { ...next.turnPhase, lastEventMs: command.at };
                }
            }
            return {
                state: next,
                events: [
                    {
                        type: "context-compacted",
                        tokensBefore: command.preTokens,
                        tokensAfter: command.postTokens,
                        source: "real",
                        trigger: command.trigger,
                        durationMs: command.durationMs,
                        frameTimestamp: command.frameTimestamp,
                    },
                ],
            };
        }

        case "DetailsToggle": {
            return {
                state: { ...state, detailsOpen: !state.detailsOpen },
                events: [],
            };
        }
        case "DetailsExpand": {
            if (state.detailsOpen) return { state, events: [] };
            return {
                state: { ...state, detailsOpen: true },
                events: [],
            };
        }
        case "DetailsCollapse": {
            if (!state.detailsOpen) return { state, events: [] };
            return {
                state: { ...state, detailsOpen: false },
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
            waitingReason: undefined,
            retryAfterMs: undefined,
        };
    } else if (
        next.turnPhase.kind === "Submitting" ||
        next.turnPhase.kind === "Idle" ||
        next.turnPhase.kind === "Disconnected" ||
        (next.turnPhase.kind === "Done" && next.turnPhase.outcome === "completed")
    ) {
        // codex P1 on #987: `StreamSubscribe` fires once at mount, so
        // subsequent turns enter Submitting and then no transition
        // arm ever advances them to Streaming — chunks/tokens/tools
        // arrive but phase stayed stuck. First stream activity from
        // Submitting is the actual "stream is producing" signal.
        //
        // Also promote from Idle / Disconnected: live tool/token activity
        // (this helper only runs for live stream events, never history) is
        // proof the agent is working. After a stream drop + resubscribe
        // (agent kill+respawn during a long upstream stall) the phase lands
        // in Idle/Disconnected; without this the working indicator stays off
        // while output streams.
        // Done.completed is included: session_end fires after every model API
        // round, so Done.completed can mean "first round of a multi-round
        // tool-continuation finished" — re-enter Streaming on next activity.
        // Done.errored/stopped/interrupted excluded: a late stray event must
        // not revive the indicator on a failed/aborted turn.
        // Interrupting is intentionally excluded (user is stopping).
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

/**
 * Sum a just-completed turn's merged stats into the pane's running total.
 * Unlike mergeStats (which replaces sessionStats per turn), this adds —
 * missing numeric fields are treated as 0, num_turns falls back to 1 per
 * completed turn so a CLI that omits it still counts as one query.
 * See SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md.
 */
function accumulateStats(
    totals: AgentPaneState["sessionTotals"],
    merged: AgentPaneState["sessionStats"],
): AgentPaneState["sessionTotals"] {
    if (!merged) return totals;
    return {
        cost_usd: (totals?.cost_usd ?? 0) + (merged.cost_usd ?? 0),
        duration_ms: (totals?.duration_ms ?? 0) + (merged.duration_ms ?? 0),
        input_tokens: (totals?.input_tokens ?? 0) + (merged.input_tokens ?? 0),
        output_tokens: (totals?.output_tokens ?? 0) + (merged.output_tokens ?? 0),
        num_turns: (totals?.num_turns ?? 0) + (merged.num_turns ?? 1),
    };
}
