// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which agents are open anywhere in this AgentMux instance, and where.
 *
 * A renderer's own pane registry (`getOpenDefinitionMap`) only holds the
 * panes in its window, so an agent torn off into a floating window, or open
 * in another window, looked closed to My agents: clicking it reattached
 * into a second pane that could never run. srv's `agent.open-panes` knows
 * every agent pane in the instance; this merges it with the local map.
 * SPEC_AGENT_SAME_PROCESS_DUPLICATE_PANE_RECOVERY_2026_10_01.md §4.0.
 */

import type { AgentOpenPane } from "@/app/store/rpc-api";

export interface OpenAgentLocation {
    blockId: string;
    /** The pane is in the window asking. */
    here: boolean;
    /** Short phrase completing "<agent> is already …", e.g. "open in another window". */
    label: string;
}

/**
 * definitionId → an open blockId, instance-wide. A pane in this window wins
 * over one elsewhere (switching to it is cheaper), and the local map wins
 * over srv's: it has a just-opened pane before srv reports it.
 */
export function mergeOpenDefinitions(
    local: ReadonlyMap<string, string>,
    panes: readonly AgentOpenPane[],
    myWindowId: string
): Map<string, string> {
    const result = new Map<string, string>();
    for (const p of panes) {
        if (!p.agent_id) continue;
        if (!result.has(p.agent_id) || isHere(p, myWindowId)) result.set(p.agent_id, p.block_id);
    }
    for (const [defId, blockId] of local) result.set(defId, blockId);
    return result;
}

/** Where each open definition is, for the row's wording. */
export function openAgentLocations(
    local: ReadonlyMap<string, string>,
    panes: readonly AgentOpenPane[],
    myWindowId: string
): Map<string, OpenAgentLocation> {
    const byBlock = new Map(panes.map((p) => [p.block_id, p] as const));
    const result = new Map<string, OpenAgentLocation>();
    for (const [defId, blockId] of mergeOpenDefinitions(local, panes, myWindowId)) {
        const pane = byBlock.get(blockId);
        const here = local.get(defId) === blockId || (pane != null && isHere(pane, myWindowId));
        let label: string;
        if (here) {
            label = pane?.tab_name ? `open in another pane (${pane.tab_name})` : "open in another pane";
        } else {
            label = "open in another window";
        }
        result.set(defId, { blockId, here, label });
    }
    return result;
}

function isHere(p: AgentOpenPane, myWindowId: string): boolean {
    return !!myWindowId && p.window_ids.includes(myWindowId);
}
