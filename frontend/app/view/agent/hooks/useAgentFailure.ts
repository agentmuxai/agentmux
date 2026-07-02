// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure — owns the agent-pane failure-recovery surface.
 *
 * Subscribes to the per-block `agentfailure` wave event (the classified
 * `AgentFailure` from a non-zero exit), holds the transient view state
 * (expanded body, retrying, the 5-second auto-retry countdown), and exposes a
 * `<PaneRow>` descriptor via `failureToRow`. The actual recovery *effects*
 * (re-run the turn, re-auth, open Armory) are passed in by the caller so
 * this hook stays presentation-only.
 *
 * Auto-retry: for transient classes (rate-limit / overload / network) a 5 s
 * countdown arms; clicking Retry fires immediately, reaching 0 fires
 * automatically, Dismiss cancels — capped at 2 auto-retries (5 s → 10 s).
 *
 * Spec: docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §4–§6.
 */

import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import * as WOS from "@/app/store/wos";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { failureToRow, isTransient, type FailureRow } from "../failure/failure-accessory";

const AUTO_RETRY_BACKOFF_S = [5, 10] as const; // then manual-only

export interface UseAgentFailureOptions {
    blockId: string;
    /** Re-run the failed turn (re-send the last user message). */
    onRetry: () => void;
    /** Re-authenticate this agent's provider account (P2). */
    onLoginAgain: () => void;
    /** Seed this agent from the user's existing global Claude login (§5.5). */
    onUseExistingLogin: () => void;
    /** Open a real terminal window for browser-based OAuth (Claude v2.1.x). */
    onLoginViaTerminal: () => void;
    /** Open Armory → Accounts. */
    onTrustCenter: () => void;
    /** Start a fresh agent session (context-window overflow recovery). */
    onNewSession: () => void;
    /** True when the provider supports seed-from-global (Claude). Promotes
     *  "Use existing login" to primary in the auth failure banner. */
    canSeed?: () => boolean;
}

export interface UseAgentFailureResult {
    /** The PaneRow descriptor for the current failure, or null when clear. */
    row: Accessor<FailureRow | null>;
}

export function useAgentFailure(opts: UseAgentFailureOptions): UseAgentFailureResult {
    const [failure, setFailure] = createSignal<AgentFailure | null>(null);
    const [expanded, setExpanded] = createSignal(false);
    const [retrying, setRetrying] = createSignal(false);
    const [autoRetryIn, setAutoRetryIn] = createSignal<number | null>(null);

    let countdown: ReturnType<typeof setInterval> | undefined;
    let autoRetries = 0;
    // True after a turn emits `done` but before its success/failure verdict is
    // known. The verdict is decided by whether an `agentfailure` follows the
    // `done`: if the NEXT event is another turn's `running` with this still set,
    // the prior turn completed with no failure → it succeeded. Used to reset the
    // auto-retry budget on genuine success without trusting the exit code (a
    // throttled transient turn exits 0 too — see the controllerstatus handler).
    let awaitingVerdict = false;

    const cancelCountdown = () => {
        if (countdown) clearInterval(countdown);
        countdown = undefined;
        setAutoRetryIn(null);
    };

    const clear = () => {
        cancelCountdown();
        setFailure(null);
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
        opts.onRetry();
        // The next turn's lifecycle clears the row (a new turn = no failure);
        // also clear locally so the banner goes away immediately. Keep
        // `autoRetries` — an auto-fired retry stays within the episode's cap.
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
        if (pf) setFailure(pf);

        const unsubFailure = waveEventSubscribe({
            eventType: WpsEvent.AgentFailure,
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) return;
                // This turn's verdict is decided: it FAILED. (Clear the pending
                // flag so the next `running` doesn't misread it as a success.)
                awaitingVerdict = false;
                cancelCountdown();
                setExpanded(false);
                setRetrying(false);
                setFailure(f);
                if (isTransient(f.code)) armAutoRetry();
            },
        });
        const unsubStatus = waveEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const data = (event as any)?.data;
                const procStatus = data?.shellprocstatus;
                if (procStatus === "done") {
                    // A turn finished. Whether it SUCCEEDED or FAILED is decided
                    // by whether an `agentfailure` follows (it always does for a
                    // failure, and arrives right after `done`). We can't use the
                    // exit code: a throttled transient turn is classified from an
                    // error `result` frame and exits 0, emitting `done(exit 0)`
                    // BEFORE its `agentfailure` (subprocess.rs) — so exit 0 does
                    // NOT mean success. Defer the verdict to the next event.
                    awaitingVerdict = true;
                } else if (procStatus === "running") {
                    if (failure()) {
                        // A new turn starting while a failure row is still
                        // visible = the user composed a fresh message (the Retry
                        // button goes through doRetry, which already cleared the
                        // row). Fresh task → clear the row + reset the budget.
                        endEpisode();
                    } else if (awaitingVerdict) {
                        // The previous turn emitted `done` and NO `agentfailure`
                        // followed → it SUCCEEDED. The episode (if any) is over,
                        // so restore the full auto-retry budget; a later
                        // unrelated transient failure must get its own 2
                        // auto-retries. A *failing* turn clears `awaitingVerdict`
                        // in the agentfailure handler above, so the cascade keeps
                        // its count and still caps at 2. Spec §6.
                        autoRetries = 0;
                    }
                    awaitingVerdict = false;
                }
            },
        });
        onCleanup(() => {
            unsubFailure();
            unsubStatus();
            cancelCountdown();
        });
    });

    const row = (): FailureRow | null => {
        const f = failure();
        if (!f) return null;
        return failureToRow(
            f,
            { expanded: expanded(), autoRetryIn: autoRetryIn(), retrying: retrying(), canSeed: opts.canSeed?.() },
            {
                retry: doRetry,
                loginAgain: opts.onLoginAgain,
                useExistingLogin: opts.onUseExistingLogin,
                loginViaTerminal: opts.onLoginViaTerminal,
                trustCenter: opts.onTrustCenter,
                newSession: opts.onNewSession,
                toggleDetails: () => setExpanded((v) => !v),
                dismiss: endEpisode,
            },
        );
    };

    return { row };
}
