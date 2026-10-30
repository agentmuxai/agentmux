// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * failure-verdict — what the failure row shows is the failure judged against what is true now.
 *
 * The record (`AgentFailure` and when it landed) stays the cause. This pure function, with no hooks
 * and no store, says whether the cause still holds: `active` (show the row as it is), `healed`
 * (the cause is over: clear the row, silently) or `lapsed` (a transient failure whose retries are
 * spent: a muted note, not an error). It is the single source for "may a failure row be hidden":
 * a row is never hidden while its verdict is `active`.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_FAILURE_ROW_FOLLOWS_LIVE_STATE_2026_10_29.md §3.
 */

import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import type { AgentRuntimeState } from "@/types/rpc/AgentRuntimeState";

export type FailureVerdict = "active" | "healed" | "lapsed";

/** What srv says about the agent's account: it can't work for want of one, it can, or nothing is known. */
export type AccountState = "limited" | "signed_out" | "can_work" | "unknown";

/** The account state of an agent's runtime state. A stopped, starting or errored agent says nothing about its account. */
export function accountStateOf(state: AgentRuntimeState | undefined): AccountState {
    switch (state) {
        case "limited":
            return "limited";
        case "signed_out":
            return "signed_out";
        case "idle":
        case "working":
        case "waiting_on_user":
            return "can_work";
        default:
            return "unknown";
    }
}

/** What is true now, from the places that already know it. */
export interface FailureLive {
    /** srv's state for this agent (the `agentruntime` feed). */
    account: AccountState;
    /** When this pane last completed a turn (`turn-ended`, outcome `completed`), if it has. */
    completedTurnAt: number | null;
    /** A transient failure whose auto-retry ladder has ended (or never ran in this pane). */
    ladderEnded: boolean;
    /** Whether the provider's API answers now, when srv tracks it. */
    apiReachable?: boolean | null;
}

/** The failure as the pane holds it. */
export type JudgedFailure = Pick<PaneFailure, "data" | "at" | "turnAttempted">;

/** Classes about the account: only these may offer account actions, and only while `active`. */
export function isAccountCause(code: AgentFailure["code"]): boolean {
    return code === "auth" || code === "usage_limit";
}

export function failureVerdict(f: JudgedFailure, live: FailureLive, now: number): FailureVerdict {
    // The pre-launch "Not signed in" row is the pane's own: it ends with its login (`decideSyntheticRow`).
    if (f.turnAttempted === false) return "active";
    const completedSince = live.completedTurnAt != null && live.completedTurnAt > f.at;
    switch (f.data.code) {
        case "auth":
            return completedSince || live.account === "can_work" ? "healed" : "active";
        case "usage_limit": {
            const resetPassed = f.data.resetsAtMs != null && f.data.resetsAtMs <= now;
            return completedSince || resetPassed || live.account === "can_work" ? "healed" : "active";
        }
        case "rate_limited":
        case "overloaded":
            return completedSince ? "healed" : live.ladderEnded ? "lapsed" : "active";
        case "network":
            return completedSince || live.apiReachable === true ? "healed" : live.ladderEnded ? "lapsed" : "active";
        default:
            return completedSince ? "healed" : "active";
    }
}

/** "3 min", "2 h", "1 d": how long ago, for the lapsed note. */
export function agoLabel(at: number, now: number): string {
    const minutes = Math.max(0, Math.round((now - at) / 60_000));
    if (minutes < 1) return "just now";
    if (minutes < 60) return `${minutes} min ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 48) return `${hours} h ago`;
    return `${Math.round(hours / 24)} d ago`;
}
