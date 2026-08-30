// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useTurnLifecycle — "the turn is over" finalization plus the timers/watchdog
 * that drive it: the process-exit grace timer, the Esc stop-fallback timer,
 * and the stuck-stream watchdog tick. All three eventually call the same
 * `finalizeTurn`, which is returned so the caller's NDJSON parse loop can
 * also invoke it directly on a real `session_end` event.
 *
 * `finalizeTurn`'s "Interrupted by user" row is pushed into the shared
 * `StreamFlushQueue` — never dispatched or flushed independently. A second
 * independent RAF/`batch()` here would reintroduce the reconcileArrays/
 * replaceChild crash documented in RETRO_REPLACECHILD_CRASH_2026-06-06.md;
 * see stream-flush-queue.ts's module doc for the full rationale.
 *
 * Called directly from inside the caller's `onMount` (this hook does not
 * open its own nested `onMount`) so `createEffect`/`setInterval`/
 * `onCleanup` register against the same owner and the same mount pass as
 * the original inline code.
 */

import { createEffect, onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import * as WOS from "@/app/store/wos";
import { recordTurn } from "@/store/token-usage";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { SUBMIT_TIMEOUT_MS, type TurnPhase } from "@/app/store/agent-pane-state/types";
import type { SignalPair } from "../state";
import type { DocumentNode, SessionStats } from "../types";
import type { StreamFlushQueue } from "../stream-flush-queue";

/**
 * Watchdog tick rate. Every 5s the hook dispatches a StreamWatchdogTick
 * to the pane-state reducer; the reducer compares against
 * `STUCK_THRESHOLD_MS` (45s) and emits a `stream-stuck` event when the
 * subscribed stream has been silent that long. Issue #728 gap 3.
 */
const WATCHDOG_INTERVAL_MS = 5_000;

export interface UseTurnLifecycleOptions {
    blockId: string;
    model: AgentPaneModel;
    turnPhaseAtom: SignalPair<TurnPhase>;
    provider?: string;
    queue: StreamFlushQueue;
    /** Flushes the stream-parser's text/thinking accumulators. Called at the start of `finalizeTurn`. */
    flushParserPending: () => void;
    hasNodeId: (id: string) => boolean;
    addNodeId: (id: string) => void;
}

export interface UseTurnLifecycleResult {
    finalizeTurn: (stats: SessionStats | null) => void;
}

export function useTurnLifecycle(opts: UseTurnLifecycleOptions): UseTurnLifecycleResult {
    const [getTurnPhase] = opts.turnPhaseAtom;

    /**
     * Shared finalization for "the turn is over." Called by both the
     * real `session_end` event from the CLI's result line AND the
     * fallback timer armed when the user presses Esc (killing the
     * subprocess prevents it from emitting its own result event).
     *
     * If `turnPhase.kind === "Interrupting"` when this runs, the
     * ending was user-initiated — append a visible "⏹ Interrupted
     * by user" row to the document so the user has durable
     * confirmation that the stop landed.
     */
    const finalizeTurn = (stats: SessionStats | null) => {
        opts.flushParserPending();
        // Snapshot live turn tokens into SessionStats before nulling
        // the live signal. The Worked footer reads from sessionStats
        // (since turnTokens is cleared on session_end); without this
        // merge the headline tokens-in-Worked feature shows nothing.
        // Per PR #549 reagent/codex P1.
        //
        // PR G: turn-tokens are read from the reducer snapshot
        // instead of a dedicated signal accessor — same source of
        // truth, fewer props threaded into the hook.
        // Prefer the result event's turn-total usage. The live
        // turnTokens hold only the last message_start/message_delta
        // (TokensIn/TokensOut overwrite, not accumulate), so they
        // undercount multi-call turns; fall back to them only when
        // session_end carries no usage (e.g. providers without a
        // token-bearing result line).
        const liveTokens = paneSnapshot(opts.blockId)?.turnTokens ?? null;
        const statsTokens =
            stats && (stats.input_tokens != null || stats.output_tokens != null)
                ? {
                    input: stats.input_tokens ?? 0,
                    output: stats.output_tokens ?? 0,
                    // Same breakdown carried alongside input/output — see
                    // TurnTokens'/SessionStats' doc comments in ../types.ts.
                    freshInput: stats.fresh_input_tokens,
                    cacheCreation: stats.cache_creation_input_tokens,
                    cacheRead: stats.cache_read_input_tokens,
                }
                : null;
        const tokens = statsTokens ?? liveTokens;
        // Aggregate the completed turn's tokens into the global
        // session-local token-usage store so the status bar's
        // indicator + breakdown popover stay up to date. Guarded
        // against double-counting by recordTurn's own no-op-on-zero
        // check — see SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
        //
        // This is the only recordTurn call site with real pane/agent
        // identity in scope — pass it through so the status bar's
        // breakdown can group by agent instead of just by provider. A
        // one-shot (non-reactive) block-meta read is enough here: we
        // only need the name at this exact instant, not a live
        // subscription. See SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md.
        //
        // Called even when `tokens` is null — a stats-only session_end
        // (cost_usd/num_turns reported, no token usage at all) is a real
        // shape the Claude translator emits; recordTurn's own no-op
        // check handles the case where there's truly nothing to record.
        // Codex P2 on PR #2849 — this used to be gated on `tokens` being
        // truthy, silently dropping that cost/turn-count data.
        if (opts.provider) {
            const agentName =
                WOS.getObjectValue<Block>(WOS.makeORef("block", opts.blockId))?.meta?.agentName
                ?? opts.blockId.slice(0, 7);
            recordTurn(opts.provider, tokens, {
                blockId: opts.blockId,
                agentName,
                costUsd: stats?.cost_usd,
                numTurns: stats?.num_turns,
            });
        }
        // Detect user-initiated stop via the reducer's turn phase.
        // PR G: replaces the legacy `stoppingAtom` getter — the
        // predicate is exactly `turnPhase.kind === "Interrupting"`
        // since RequestStop is the only command that enters
        // Interrupting and TurnEnd is the next transition out.
        // The reducer's TurnEnd handler does the cross-atom cleanup
        // in one shot: merges live tokens into stats, clears
        // tool/tokens, and transitions the phase to Done.
        const wasStopping = getTurnPhase().kind === "Interrupting";
        opts.model.dispatchPane({ type: "TurnEnd", stats });
        if (wasStopping) {
            const interruptedNode: DocumentNode = {
                type: "markdown",
                id: `interrupted-${Date.now()}`,
                content: "⏹ _Interrupted by user_",
                timestamp: Date.now(),
            };
            if (!opts.hasNodeId(interruptedNode.id)) {
                opts.addNodeId(interruptedNode.id);
                opts.queue.pushNewNode(interruptedNode);
                opts.queue.scheduleFlush();
            }
        }
    };

    // Process-exit grace-period: when the backend subprocess exits
    // (`ControllerStatus: done`), give 1.5 s for any buffered
    // `session_end` to drain through the IPC. If the phase is still
    // working after that window, the process crashed without emitting
    // `session_end` — force StreamUnsubscribe so "Working…" clears
    // and the Disconnected state surfaces the AgentFailure banner.
    //
    // Clean exit: `session_end` → `finalizeTurn` → `TurnEnd` puts the
    // phase in Done before the timer fires → no-op.
    // Persistent mode: the process never exits between turns, so
    // `ControllerStatus: done` only fires on crash or session teardown.
    // Auto-retry: `ControllerStatus: running` cancels any pending timer.
    let procExitGraceTimer: number | null = null;
    const procExitUnsub = waveEventSubscribe({
        eventType: WpsEvent.ControllerStatus,
        scope: WOS.makeORef("block", opts.blockId),
        handler: (event) => {
            const status = (event as any)?.data?.shellprocstatus;
            if (status === "running") {
                if (procExitGraceTimer != null) {
                    clearTimeout(procExitGraceTimer);
                    procExitGraceTimer = null;
                }
                return;
            }
            if (status !== "done") return;
            if (procExitGraceTimer != null) return; // already armed
            procExitGraceTimer = window.setTimeout(() => {
                procExitGraceTimer = null;
                const phase = paneSnapshot(opts.blockId)?.turnPhase?.kind;
                if (phase === "Streaming" || phase === "Submitting") {
                    const at = Date.now();
                    // StreamUnsubscribe transitions Streaming → Disconnected,
                    // clearing "Working...", but also nulls lastEventMs in the
                    // reducer — which would gate TurnStart and StreamFlushObserved
                    // for any recovery turn (failure-banner Retry or auto-retry).
                    // Immediately re-dispatch StreamSubscribe to restore lastEventMs
                    // while keeping the file subscription live. Net phase: Idle
                    // (Disconnected → Idle via StreamSubscribe). The AgentFailure
                    // banner drives the crash UX independently of turn phase.
                    opts.model.dispatchPane({ type: "StreamUnsubscribe", at });
                    opts.model.dispatchPane({ type: "StreamSubscribe", at });
                }
            }, 1500);
        },
    });
    onCleanup(() => {
        procExitUnsub();
        if (procExitGraceTimer != null) {
            clearTimeout(procExitGraceTimer);
            procExitGraceTimer = null;
        }
    });

    // Cancel the crash-recovery timer when a new turn is submitted.
    // Gated on Submitting only — NOT Streaming — because StreamFlushObserved
    // replaces the Streaming phase object with a fresh reference even for the
    // dying turn's buffered output, which would spuriously cancel the timer
    // before it can fire. Submitting is only entered via TurnStart (a real
    // new turn), so it is safe to cancel here.
    createEffect(() => {
        const kind = getTurnPhase().kind;
        if (kind === "Submitting" && procExitGraceTimer != null) {
            clearTimeout(procExitGraceTimer);
            procExitGraceTimer = null;
        }
    });

    // Fallback timer: if the user presses Esc and the CLI doesn't
    // emit `session_end` within 1.5s (normal for a killed subprocess
    // — TerminateProcess skips any final output), run the same
    // finalization locally so the UI doesn't hang on "Stopping…".
    //
    // PR G: previously gated on `getStopping()` (legacy boolean);
    // now gated on `turnPhase.kind === "Interrupting"`. Same edge
    // semantics — the reducer transitions into Interrupting on
    // RequestStop and out on TurnEnd / disconnect.
    let stopFallbackTimer: number | null = null;
    createEffect(() => {
        const stopping = getTurnPhase().kind === "Interrupting";
        if (stopFallbackTimer != null) {
            clearTimeout(stopFallbackTimer);
            stopFallbackTimer = null;
        }
        if (stopping) {
            stopFallbackTimer = window.setTimeout(() => {
                stopFallbackTimer = null;
                if (getTurnPhase().kind === "Interrupting") {
                    finalizeTurn(null);
                }
            }, 1500);
        }
    });
    onCleanup(() => {
        if (stopFallbackTimer != null) {
            clearTimeout(stopFallbackTimer);
            stopFallbackTimer = null;
        }
    });

    // Submit-ack fallback timer (issue #728 gap 2 / spec §8's
    // `SubmitTimeoutElapsed` contract). `TurnStart` puts the phase in
    // `Submitting` and the reducer emits `schedule-submit-timeout` on
    // entry, but nothing ever consumed that event to arm a real timer —
    // PR D (#994, 2026-05-23) shipped the reducer half only and
    // explicitly deferred "dispatch-side setTimeout in useAgentStream /
    // model layer" to a follow-up that never landed (confirmed via a
    // repo-wide grep finding zero consumers outside reducer/types/tests,
    // 2026-08-14). Without this, a turn whose backend ack is lost
    // (dropped RPC, network blip, crash before first output) left the
    // pane in `Submitting` — and therefore "Working…" — forever, with
    // no recovery path at all. Mirrors `stopFallbackTimer` above exactly
    // (reactive effect on `turnPhase.kind`, not an event-listener on
    // `schedule-submit-timeout`, which sidesteps needing a second wiring
    // mechanism entirely) but dispatches the reducer's own
    // `SubmitTimeoutElapsed` command instead of calling `finalizeTurn`
    // directly, so the transition goes through the documented,
    // already-tested `Submitting → Done.errored` contract (reducer.ts's
    // `SubmitTimeoutElapsed` arm no-ops if the phase already moved off
    // Submitting by the time this fires — safe by construction, same
    // guard `stopFallbackTimer` relies on). See
    // docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md §7.
    //
    // Deliberately scoped to "did the backend ever acknowledge receiving
    // this message" — NOT "did the model produce its first token" (codex
    // P1 on PR #2575): `usePendingMessageAcceptance.ts` intentionally
    // leaves an idle send in `Submitting` after `AgentMessageAccepted`
    // arrives (its own comment: re-dispatching TurnStart there would
    // regress Streaming → Submitting and re-arm this exact timer
    // unnecessarily), and a backend-accepted turn can legitimately take
    // well over 30s to produce its first token (large context, a long
    // agentic tool chain) — that window is already independently bounded
    // server-side by `HealthMonitor`'s own Stalled(30s)/Dead(120s)
    // silence detection once the backend marks the turn active, which
    // surfaces as a real `AgentFailure` → `FailureObserved` → this same
    // `Done.errored` transition through an already-tested, unrelated
    // path. A blind 30s bound here would misfire on exactly that healthy
    // case — and since neither `StreamFlushObserved` nor `bumpEvent`
    // re-promote an `errored` `Done` phase, the eventual real response
    // would arrive with the lifecycle stuck errored and the UI free to
    // start an overlapping second turn. `acceptedUnsub` below clears the
    // timer the instant the backend proves it's alive for this pane,
    // narrowing this timeout to only the "message never reached the
    // backend at all" case it exists to catch.
    //
    // reagentx P1 (round 2): a per-hook-lifetime `AgentMessageAccepted`
    // listener (the original shape of this fix) carries no correlation to
    // WHICH `Submitting` episode it's really for — a stale/late accepted
    // event for an EARLIER message, delivered after that episode already
    // timed out and a NEWER, still-genuinely-unacknowledged `TurnStart`
    // began (e.g. the backend's own retried WPS push, or a reconnect
    // backlog replay, landing well after the client already gave up and
    // the user retried), would incorrectly disarm the NEWER episode's
    // timer — silently reintroducing the exact stuck-Submitting bug this
    // fix exists to close. `usePendingMessageAcceptance.ts` guards this by
    // matching the event's `message_id` against its own pending-queue
    // entry; that queue isn't available here, and `Submitting` carries no
    // message_id to match against directly (a comparable timestamp-based
    // guard was tried and rejected — it only tracks "whatever the latest
    // armed episode is," which a stale event for ANY prior episode would
    // still match against once a new one has armed, not an actual fix).
    // Scoping BOTH the timer handle and the subscription to a fresh local
    // closure per episode — armed inside the effect body, torn down via
    // SolidJS's per-run `onCleanup` the instant the phase changes — is
    // structurally correct instead: once episode A's run of this effect is
    // cleaned up (the phase changed away from Submitting), A's listener is
    // unsubscribed and A's timer handle is cleared, so a stale event for A
    // literally has nothing left to disarm by the time episode B's own
    // fresh timer/listener pair is live. Deliberately a per-run LOCAL
    // variable, not one shared across the whole hook — if `waveEventSubscribe`
    // ever delivered to an already-unsubscribed handler (a transport-level
    // race this hook has no control over), that stale handler's closure
    // would still only ever see episode A's own (already cleared or
    // already-fired) timer handle, never episode B's, so it cannot affect a
    // different episode's timer even in that adversarial case. No timestamp
    // comparison, no message_id needed.
    createEffect(() => {
        const phase = getTurnPhase();
        if (phase.kind !== "Submitting") return;
        let timer: number | null = window.setTimeout(() => {
            timer = null;
            if (getTurnPhase().kind === "Submitting") {
                opts.model.dispatchPane({ type: "SubmitTimeoutElapsed", at: Date.now() });
            }
        }, SUBMIT_TIMEOUT_MS);
        const acceptedUnsub = waveEventSubscribe({
            eventType: WpsEvent.AgentMessageAccepted,
            scope: WOS.makeORef("block", opts.blockId),
            handler: () => {
                if (timer != null) {
                    clearTimeout(timer);
                    timer = null;
                }
            },
        });
        onCleanup(() => {
            acceptedUnsub();
            if (timer != null) {
                clearTimeout(timer);
                timer = null;
            }
        });
    });

    // Stuck-stream watchdog (issue #728 gap 3). The reducer evaluates
    // each tick against `lastEventMs` and emits a `stream-stuck`
    // event when the gap exceeds `STUCK_THRESHOLD_MS`. The interval
    // cleans up via the same effect cleanup as the subscription.
    const watchdogId = setInterval(() => {
        opts.model.dispatchPane({
            type: "StreamWatchdogTick",
            nowMs: Date.now(),
        });
    }, WATCHDOG_INTERVAL_MS);
    onCleanup(() => clearInterval(watchdogId));

    // Visibility catch-up tick — belt-and-suspenders for the interval
    // above. A confirmed live incident
    // (docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md
    // §3-4) found a pane stuck `Streaming`/`toolsActive:0` for 29+
    // minutes after a stray/late `StreamFlushObserved` re-promotion —
    // the exact shape `StreamWatchdogTick`'s 180s unconditional recovery
    // exists to catch — with zero evidence the interval above ever
    // ticked during that window (component-unmount and a reducer-logic
    // bug were both ruled out; the leading remaining explanation is
    // renderer/window-level timer throttling for a backgrounded pane in
    // this app's multi-window CEF architecture, not fully confirmed).
    // `document.visibilitychange` fires reliably even for throttled
    // content — it's the signal browsers use to drive that throttling
    // in the first place — so firing one extra tick the instant
    // visibility returns recovers a stuck pane the moment a human next
    // looks at the app, without waiting on the interval to resume.
    // Safe/cheap: `StreamWatchdogTick`'s recovery math is already
    // wall-clock-based (`nowMs - lastEventMs`), not tick-counted, so an
    // extra or redundant tick is a same-reference no-op for every pane
    // that isn't actually stuck.
    const onVisibilityChange = () => {
        // Logged on EVERY invocation (both directions), not just the
        // "visible" branch below that dispatches the catch-up tick — this
        // is the only direct evidence that `visibilitychange` fired at all
        // for this pane's window, which the fix above depends on but which
        // no log line confirmed before this. Without it, a future incident
        // is back to inferring from the resulting (indistinguishable from
        // a regular 5s interval tick) `StreamWatchdogTick` dispatch, same
        // gap the Aug 14 report had to reason around. See
        // docs/specs/SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md.
        console.info(
            "[wave-turn]",
            `pane=${opts.blockId.slice(0, 7)}`,
            `visibility: ${document.visibilityState === "visible" ? "hidden→visible" : "visible→hidden"}`,
        );
        if (document.visibilityState === "visible") {
            opts.model.dispatchPane({
                type: "StreamWatchdogTick",
                nowMs: Date.now(),
            });
        }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisibilityChange));

    return { finalizeTurn };
}
