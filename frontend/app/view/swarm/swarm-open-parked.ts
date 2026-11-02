// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Swarm row for an agent that runs with no pane (closing its pane, tab or window parked it) has no pane
// to focus, so a click on the row opens one instead: the same call My Agents' Open makes.

/** The block meta srv sets on a block whose agent runs with no pane. */
export const PARKED_META = "agent:parked";

/** What a parked row says on hover. */
export const PARKED_ROW_TITLE = "click to open its pane";

export function isParkedMeta(meta: Record<string, unknown> | undefined): boolean {
    return meta?.[PARKED_META] === true;
}

/** The agent a block runs, by the id `agent.open` takes: its instance when the block is bound to one, else its id. */
export function agentUidOfBlockMeta(meta: Record<string, unknown> | undefined): string {
    for (const key of ["agentInstanceId", "agentId"]) {
        const value = meta?.[key];
        if (typeof value === "string" && value !== "") return value;
    }
    return "";
}
