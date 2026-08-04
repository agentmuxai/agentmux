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
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
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
                ? { input: stats.input_tokens ?? 0, output: stats.output_tokens ?? 0 }
                : null;
        const tokens = statsTokens ?? liveTokens;
        // Aggregate the completed turn's tokens into the global
        // session-local token-usage store so the status bar's
        // indicator + breakdown popover stay up to date. Guarded
        // against double-counting by recordTurn's own no-op-on-zero
        // check — see SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
        if (opts.provider && tokens) {
            recordTurn(opts.provider, tokens);
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

    return { finalizeTurn };
}
