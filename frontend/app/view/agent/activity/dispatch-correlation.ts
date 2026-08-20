// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ordinal-matches this pane's Agent/Task/Workflow tool_use nodes against
 * this pane's live `AgentDispatch` records, by transcript-order <-> spawn-
 * order position. There is no exact id linking a transcript tool_use block
 * to the dispatch it spawned (`AgentDispatch`/`SubAgent` carry no
 * `tool_use_id` — confirmed, see docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md
 * and the transcript-card design work referencing it), so this reconstructs
 * a best-effort mapping instead.
 *
 * All-or-nothing per pane, not best-effort per node: a wrong card (showing
 * the wrong dispatch's status next to a tool call) is worse than no card,
 * so any ambiguity for ANY node in the pane blanks the whole pane's map
 * rather than guessing. Known, accepted gaps that fall back this way:
 *   - multiple Agent/Task calls spawned in parallel within a single
 *     assistant turn (array order isn't guaranteed to equal spawn order);
 *   - a dispatch whose member data has aged out of `ListActive` (no
 *     `spawned_at` to order by);
 *   - older transcript history not yet paginated into `documentNodes`.
 */

import type { AgentDispatch, ActiveSubagent } from "../../swarm/swarm-model";
import type { DocumentNode, ToolNode } from "../types";

const DISPATCH_TOOL_KINDS: ReadonlySet<ToolNode["tool"]> = new Set(["Agent", "Task", "Workflow"]);

function isDispatchToolNode(n: DocumentNode): n is ToolNode {
    return n.type === "tool" && DISPATCH_TOOL_KINDS.has((n as ToolNode).tool);
}

/**
 * @param blockId the pane's block id (dispatches and subagents are scoped
 *   to `parent_block_id`, which is the whole pane, not a specific tool call)
 * @param documentNodes this pane's current transcript, in order
 * @param subagents `allSubagentsAtom()` — used only to reconstruct spawn
 *   order (`AgentDispatch` itself carries no spawn timestamp, only
 *   `last_event_at`, and `ListDispatches`/`ListActive` both sort by
 *   `last_event_at` descending — not spawn order)
 * @param dispatches `allDispatchesAtom()`
 * @returns tool_use_id (`ToolNode.id`) -> its matched `AgentDispatch`, or an
 *   empty map when the pane's match isn't confident enough to trust
 */
export function correlateDispatchesForBlock(
    blockId: string,
    documentNodes: readonly DocumentNode[],
    subagents: readonly ActiveSubagent[],
    dispatches: readonly AgentDispatch[]
): Map<string, AgentDispatch> {
    const result = new Map<string, AgentDispatch>();

    const toolNodes = documentNodes.filter(isDispatchToolNode);
    if (toolNodes.length === 0) return result;

    // Effective spawn time per dispatch_id = earliest known member
    // spawned_at, scoped to this pane.
    const spawnOrderOf = new Map<string, number>();
    for (const s of subagents) {
        if (s.parent_block_id !== blockId) continue;
        const prev = spawnOrderOf.get(s.dispatch_id);
        if (prev === undefined || s.spawned_at < prev) spawnOrderOf.set(s.dispatch_id, s.spawned_at);
    }

    const blockDispatches = dispatches.filter((d) => d.parent_block_id === blockId);
    const orderable = blockDispatches.filter((d) => spawnOrderOf.has(d.dispatch_id));

    // Confidence gate — disqualifies the WHOLE pane:
    //  - a dispatch exists for this pane but isn't orderable (its member
    //    data aged out of ListActive) — order downstream of it can't be
    //    trusted;
    //  - counts disagree with the transcript's tool-node count — parallel
    //    same-turn spawns, a dispatch pruned entirely from ListDispatches,
    //    or unpaginated older history all show up here as a mismatch, which
    //    is the safe direction to fail in (undercount -> no match, never a
    //    wrong match).
    if (blockDispatches.length !== orderable.length) return result;
    if (toolNodes.length !== orderable.length) return result;

    const sorted = [...orderable].sort((a, b) => spawnOrderOf.get(a.dispatch_id)! - spawnOrderOf.get(b.dispatch_id)!);
    for (let i = 0; i < toolNodes.length; i++) {
        result.set(toolNodes[i].id, sorted[i]);
    }
    return result;
}
