// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which writer of the pane's failure wins. Several places raise the failure row (the backend's
 * event, a command putting back what a send cleared, the stored failure at mount, the pre-launch
 * sign-in row, the offline retry), and the reducer applies them in arrival order, so the rule has
 * to be written down and enforced where they meet: `FailureObserved` is dropped when the failure
 * already showing outranks its source.
 *
 *   live       the backend's classified failure (the `agentfailure` event): the truth, always applies
 *   command    a command's own outcome: a failed send rolled back, an auth failure put back
 *   synthetic  the pane's pre-launch "Not signed in" row: yields to any real failure
 *   seed       the failure stored in block meta, put up when the pane mounts: yields to anything live
 *   offline    a failure restored from an offline retry mark
 *
 * A source applies over a failure of the same or a lower rank. Docs: docs/specs/SPEC_AGENT_PANE_FAILURE_ROW_FOLLOWS_LIVE_STATE_2026_10_29.md.
 */

import { workingFromPhase, type AgentPaneState, type PaneFailure } from "./types";

export type FailureSource = "live" | "command" | "synthetic" | "seed" | "offline";

export const FAILURE_RANK: Record<FailureSource, number> = {
    live: 5,
    command: 4,
    synthetic: 3,
    seed: 2,
    offline: 1,
};

/** The source of a failure the pane holds; one recorded before sources existed was the backend's. */
export function sourceOf(failure: Pick<PaneFailure, "source">): FailureSource {
    return failure.source ?? "live";
}

/** The failure showing outranks `incoming`, so `incoming` must not replace it. */
export function outranks(current: PaneFailure | null, incoming: FailureSource): boolean {
    return current !== null && FAILURE_RANK[sourceOf(current)] > FAILURE_RANK[incoming];
}

/**
 * Whether the reducer drops a `FailureObserved`: the failure showing outranks its source, or it is
 * a stored failure arriving late (`seed`) after a turn began or finished since the pane mounted
 * (`since`). A running turn must not be ended by one, and a finished one is newer than it.
 */
export function failureDropped(
    state: Pick<AgentPaneState, "failure" | "turnPhase">,
    command: { source?: FailureSource; since?: number },
): boolean {
    const source = command.source ?? "live";
    if (outranks(state.failure, source)) return true;
    if (source !== "seed") return false;
    const phase = state.turnPhase;
    const finishedSince = phase.kind === "Done" && command.since !== undefined && phase.finishedAt > command.since;
    return workingFromPhase(phase) || finishedSince;
}
