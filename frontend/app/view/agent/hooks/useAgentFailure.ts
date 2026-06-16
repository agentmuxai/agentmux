// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure — owns the agent-pane failure-recovery surface.
 *
 * Subscribes to the per-block `agentfailure` wave event (the classified
 * `AgentFailure` from a non-zero exit), holds the transient view state
 * (expanded body, retrying, the 5-second auto-retry countdown), and exposes a
 * `<PaneRow>` descriptor via `failureToRow`. The actual recovery *effects*
 * (re-run the turn, re-auth, open Trust Center) are passed in by the caller so
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
import * as WOS from "@/app/store/wos";
import { failureToRow, isTransient, type FailureRow } from "../failure/failure-accessory";

const AUTO_RETRY_BACKOFF_S = [5, 10] as const; // then manual-only

export interface UseAgentFailureOptions {
    blockId: string;
    /** Re-run the failed turn (re-send the last user message). */
    onRetry: () => void;
    /** Re-authenticate this agent's provider account (P2). */
    onLoginAgain: () => void;
    /** Open Trust Center → Accounts. */
    onTrustCenter: () => void;
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

    const doRetry = () => {
        cancelCountdown();
        setRetrying(true);
        opts.onRetry();
        // The next turn's lifecycle clears the row (a new turn = no failure);
        // also clear locally so the banner goes away immediately.
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
        const unsubFailure = waveEventSubscribe({
            eventType: "agentfailure",
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) return;
                cancelCountdown();
                setExpanded(false);
                setRetrying(false);
                setFailure(f);
                if (isTransient(f.code)) armAutoRetry();
            },
        });
        // Clear the row when a NEW turn starts (manual send or a retry) — the
        // failure is from the *previous* turn, so a fresh "running" resolves it.
        const unsubStatus = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                if ((event as any)?.data?.shellprocstatus === "running" && failure()) clear();
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
            { expanded: expanded(), autoRetryIn: autoRetryIn(), retrying: retrying() },
            {
                retry: doRetry,
                loginAgain: opts.onLoginAgain,
                trustCenter: opts.onTrustCenter,
                toggleDetails: () => setExpanded((v) => !v),
                dismiss: clear,
            },
        );
    };

    return { row };
}
