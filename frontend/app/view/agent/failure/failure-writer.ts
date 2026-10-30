// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one way to raise or clear the pane's failure row. Everything that puts a failure up or takes
 * it down goes through `raiseFailure` / `clearFailure`, so each call says who it is
 * (`FailureSource`) or why (`FailureClearReason`), and the "which wins" rule lives in the reducer
 * (store/agent-pane-state/failure-precedence.ts) instead of in each caller's comment.
 * `scripts/check-failure-writers.mjs` fails on a direct `FailureObserved` / `FailureCleared`
 * dispatch anywhere else.
 */

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import type { FailureSource } from "@/app/store/agent-pane-state/failure-precedence";
import type { FailureClearReason } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";

type FailureModel = Pick<AgentPaneModel, "dispatchPane">;

export interface RaiseFailure {
    source: FailureSource;
    failure: AgentFailure;
    /** When it landed (ms); the live event's time, or the stored one. */
    at: number;
    /** False only for the pane's own pre-launch sign-in row (PaneFailure.turnAttempted). */
    turnAttempted?: boolean;
    /** For a `seed`: when the pane mounted (the reducer drops a seed a turn has overtaken). */
    since?: number;
    /** Dispatched as the pane's own doing (not a user or backend input). */
    system?: boolean;
}

/**
 * Put `failure` up. Dropped by the reducer, with no other effect, when the failure showing
 * outranks `source`. It ends a working turn when it applies (the live event's job), except a
 * `seed`, which is dropped instead of ending one.
 */
export function raiseFailure(model: FailureModel, r: RaiseFailure): void {
    const command = {
        type: "FailureObserved" as const,
        failure: r.failure,
        at: r.at,
        source: r.source,
        ...(r.turnAttempted !== undefined ? { turnAttempted: r.turnAttempted } : {}),
        ...(r.since !== undefined ? { since: r.since } : {}),
    };
    if (r.system) model.dispatchPane(command, "system");
    else model.dispatchPane(command);
}

export interface ClearFailure {
    reason: FailureClearReason;
    /** Clear it only when it is this class, decided in the reducer. Unset clears whatever is showing. */
    onlyCode?: AgentFailure["code"];
    system?: boolean;
}

/** Take the failure down. */
export function clearFailure(model: FailureModel, c: ClearFailure): void {
    const command = { type: "FailureCleared" as const, reason: c.reason, ...(c.onlyCode ? { onlyCode: c.onlyCode } : {}) };
    if (c.system) model.dispatchPane(command, "system");
    else model.dispatchPane(command);
}
