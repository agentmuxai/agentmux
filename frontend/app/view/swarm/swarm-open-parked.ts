// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Swarm row for an agent that runs with no pane (closing its pane, tab or window parked it) has no pane
// to focus, so a click on the row opens one instead: the same call My Agents' Open makes.

import { isParkedMeta, PARKED_META } from "@/app/store/agent-parked";

export { isParkedMeta, PARKED_META };

/** What a parked row says on hover. */
export const PARKED_ROW_TITLE = "click to open its pane";

/** The agent a block runs, by the id `agent.open` takes: its instance when the block is bound to one, else its id. */
export function agentUidOfBlockMeta(meta: Record<string, unknown> | undefined): string {
    for (const key of ["agentInstanceId", "agentId"]) {
        const value = meta?.[key];
        if (typeof value === "string" && value !== "") return value;
    }
    return "";
}
