// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure — owns the agent-pane failure-recovery surface.
 *
 * Subscribes to the per-block `agentfailure` wave event (the classified
 * `AgentFailure` from a non-zero exit) and forwards it into the pane
 * reducer as `FailureObserved` — which is what makes an authoritative
 * backend failure classification unconditionally end a working turn,
 * closing the "stuck Waiting after a rate-limit interruption" bug (see
 * docs/analysis/ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md).
 * The *active failure itself* (`state.failure`) now lives in
 * `AgentPaneState` — this hook reads it back via the `failure` accessor
 * passed in, instead of holding its own local copy that nothing else could
 * agree with. See docs/specs/SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md.
 *
 * What stays hook-local: the expanded-body toggle, the `retrying` flag, and
 * the 5s/10s auto-retry countdown/budget. None of these are facts anything
 * else in the app needs to agree on — they're pure view-presentation timing,
 * the same class as a `<Show>` toggle — so there's no drift risk in keeping
 * them here (same rationale the spec used to leave `expanded` local).
 *
 * The actual recovery *effects* (re-run the turn, re-auth, open Armory) are
 * passed in by the caller so this hook stays presentation-only otherwise.
 *
 * Auto-retry: for transient classes (rate-limit / overload / network) a 5 s
 * countdown arms; clicking Retry fires immediately, reaching 0 fires
 * automatically, Dismiss cancels — capped at 2 auto-retries (5 s → 10 s).
 *
 * Spec: docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §4–§6.
 */

import { createEffect, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import * as WOS from "@/app/store/wos";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { addEventListener as addPaneEventListener } from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { failureToRow, isTransient, type FailureRow } from "../failure/failure-accessory";

const AUTO_RETRY_BACKOFF_S = [5, 10] as const; // then manual-only

export interface UseAgentFailureOptions {
    blockId: string;
    /** Per-pane dispatch handle — default-safe against post-unmount races. */
    model: AgentPaneModel;
    /** Reactive read of the canonical `state.failure` (single source of
     *  truth, set by `FailureObserved` / cleared by `FailureCleared` or the
     *  next `TurnStart`). */
    failure: Accessor<PaneFailure | null>;
    /** Re-run the failed turn (re-send the last user message). */
    onRetry: () => void;
    /** Re-authenticate this agent's provider account (P2). */
    onLoginAgain: () => void;
    /** Seed this agent from the user's existing global Claude login (§5.5). */
    onUseExistingLogin: () => void;
    /** Open a real terminal window for browser-based OAuth (Claude v2.1.x). */
    onLoginViaTerminal: () => void;
    /** Open Armory → Accounts. */
    onOpenArmory: () => void;
    /** Start a fresh agent session (context-window overflow recovery). */
    onNewSession: () => void;
    /** Kill and respawn the agent's controller process (`unresponsive` only). */
    onRestart: () => void;
    /** True when the provider supports seed-from-global (Claude). Promotes
     *  "Use existing login" to primary in the auth failure banner. */
    canSeed?: () => boolean;
}

export interface UseAgentFailureResult {
    /** The PaneRow descriptor for the current failure, or null when clear. */
    row: Accessor<FailureRow | null>;
}

export function useAgentFailure(opts: UseAgentFailureOptions): UseAgentFailureResult {
    const [expanded, setExpanded] = createSignal(false);
    const [retrying, setRetrying] = createSignal(false);
    const [autoRetryIn, setAutoRetryIn] = createSignal<number | null>(null);

    let countdown: ReturnType<typeof setInterval> | undefined;
    let autoRetries = 0;
    // Set synchronously by `doRetry` right before it clears the row, and
    // consumed (reset to false) by the `state.failure` transition-effect
    // below. Distinguishes "this hook itself cleared the failure via Retry"
    // (same episode — keep the budget counting toward the cap) from "the
    // failure cleared because the user composed and sent a genuinely fresh
    // message, bypassing Retry" (a new episode — reset the budget). See the
    // effect's comment for why this replaces the previous ControllerStatus-
    // based check (reagent P1 on #1987).
    let selfInitiatedClear = false;

    const cancelCountdown = () => {
        if (countdown) clearInterval(countdown);
        countdown = undefined;
        setAutoRetryIn(null);
    };

    const clear = () => {
        cancelCountdown();
        opts.model.dispatchPane({ type: "FailureCleared" });
        setExpanded(false);
        setRetrying(false);
    };

    // End the failure *episode*: clear the row AND restore the auto-retry
    // budget. Used when the user explicitly dismisses or a genuinely new turn
    // resolves the failure — NOT by `doRetry` (an auto-retry is still part of
    // the same episode and must keep counting toward the cap, else a
    // persistently-throttled turn would auto-retry forever). Spec §6.
    const endEpisode = () => {
        autoRetries = 0;
        clear();
    };

    const doRetry = () => {
        cancelCountdown();
        setRetrying(true);
        selfInitiatedClear = true;
        opts.onRetry();
        // The next turn's lifecycle clears the row (a new turn = no failure);
        // also clear locally so the banner goes away immediately. Keep
        // `autoRetries` — an auto-fired retry stays within the episode's cap
        // (the transition-effect below sees `selfInitiatedClear` and skips
        // the reset it would otherwise apply).
        clear();
    };

    const armAutoRetry = () => {
        if (autoRetries >= AUTO_RETRY_BACKOFF_S.length) return; // capped → manual only
        const seconds = AUTO_RETRY_BACKOFF_S[autoRetries];
        autoRetries += 1;
        setAutoRetryIn(seconds);
        countdown = setInterval(() => {
            const left = (autoRetryIn() ?? 1) - 1;
            if (left <= 0) {
                doRetry();
            } else {
                setAutoRetryIn(left);
            }
        }, 1000);
    };

    onMount(() => {
        // P1.2 — Seed from persisted block meta so the recovery banner survives
        // tab switches and page reloads. Read once on mount; the WPS event
        // subscription below handles live updates for the current session.
        // (SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20 §4 P1.2)
        const persistedAtom = getBlockMetaKeyAtom(opts.blockId, "agent:last_failure");
        const pf = persistedAtom();
        // Seed the reducer's canonical `state.failure` (not a local signal —
        // see the module doc comment). The pane is freshly mounted here, so
        // there's no working turn to force-end; FailureObserved's `turnWasEnded`
        // check is false and it just records the failure.
        if (pf) opts.model.dispatchPane({ type: "FailureObserved", failure: pf, at: Date.now() });

        const unsubFailure = waveEventSubscribe({
            eventType: WpsEvent.AgentFailure,
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) {
                    // A published-but-empty agentfailure event is the
                    // backend's live-clear signal for a silent self-heal
                    // (health.rs: Dead -> anything else, reachable when late
                    // output arrives after the 120s threshold already
                    // tripped — no hysteresis on that transition). Only
                    // clear if the currently-showing failure is the class
                    // that self-heals this way — never blow away an
                    // unrelated concurrent failure (e.g. auth) that happens
                    // to be showing at the same moment. See
                    // docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §4.
                    if (opts.failure()?.data.code === "unresponsive") {
                        opts.model.dispatchPane({ type: "FailureCleared" });
                    }
                    return;
                }
                cancelCountdown();
                setExpanded(false);
                setRetrying(false);
                // Reducer-side: records state.failure AND unconditionally ends
                // a still-working turn — see FailureObserved's reducer case.
                opts.model.dispatchPane({ type: "FailureObserved", failure: f, at: Date.now() });
                if (isTransient(f.code)) armAutoRetry();
            },
        });

        // Restore the full auto-retry budget once the LAST turn genuinely
        // succeeded — a later unrelated transient failure must get its own 2
        // auto-retries, not inherit a stale count from turns ago. `turn-ended`
        // with outcome "completed" is the reducer's own authoritative verdict
        // (emitted only by the real `TurnEnd` command, driven by the CLI's own
        // session_end/result frame on stdout) — a strictly more reliable
        // signal than the previous approach of inferring success from
        // `ControllerStatus: done` NOT being followed by an `agentfailure`,
        // which depended on the underlying OS process actually exiting between
        // turns. Persistent-mode agents never do (the exact case this PR's
        // FailureObserved fix targets), so that inference could never fire for
        // them — reagent P1 on #1987. A failed turn goes through
        // `FailureObserved` instead (never emits `turn-ended`), so this can
        // never misfire on a failure.
        const unsubTurnEnded = addPaneEventListener((blockId, event) => {
            if (blockId !== opts.blockId) return;
            if (event.type === "turn-ended" && event.outcome === "completed") {
                autoRetries = 0;
            }
        });

        onCleanup(() => {
            unsubFailure();
            unsubTurnEnded();
            cancelCountdown();
        });
    });

    // Reset the budget when `state.failure` clears WITHOUT this hook having
    // initiated the clear itself — i.e. the user composed and sent a
    // genuinely fresh message while the failure row was still showing,
    // bypassing Retry entirely. `TurnStart` (reducer.ts) unconditionally
    // clears `state.failure` the instant that happens, which this effect
    // observes directly. An auto-fired or manually-clicked Retry ALSO clears
    // `state.failure` (via `clear()`), but must NOT reset the budget — same
    // episode, still capped at 2 (`armAutoRetry`) — so `doRetry` sets
    // `selfInitiatedClear` first; this effect consumes (and resets) that
    // flag on every transition it observes.
    //
    // This replaces a previous ControllerStatus-based check for the same
    // fresh-message case that fired on the backend's async `running` event —
    // but `TurnStart` clears `state.failure` synchronously, well before that
    // async event round-trips, so the old check never actually saw a
    // non-null failure by the time it ran (reagent P1 on #1987: the check
    // was dead code in practice). Watching the signal transition directly,
    // instead of re-deriving it from a separate, slower async event, is the
    // correct fix — and it works for persistent-mode agents too, unlike the
    // mechanism it replaces.
    let hadFailure = opts.failure() != null;
    createEffect(() => {
        const hasFailureNow = opts.failure() != null;
        if (hadFailure && !hasFailureNow && !selfInitiatedClear) {
            autoRetries = 0;
        }
        hadFailure = hasFailureNow;
        selfInitiatedClear = false;
    });

    const row = (): FailureRow | null => {
        const pf = opts.failure();
        if (!pf) return null;
        const f = pf.data;
        return failureToRow(
            f,
            { expanded: expanded(), autoRetryIn: autoRetryIn(), retrying: retrying(), canSeed: opts.canSeed?.() },
            {
                retry: doRetry,
                loginAgain: opts.onLoginAgain,
                useExistingLogin: opts.onUseExistingLogin,
                loginViaTerminal: opts.onLoginViaTerminal,
                openArmory: opts.onOpenArmory,
                newSession: opts.onNewSession,
                toggleDetails: () => setExpanded((v) => !v),
                dismiss: endEpisode,
                restart: opts.onRestart,
            },
        );
    };

    return { row };
}
