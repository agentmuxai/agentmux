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
 *   - a dispatch whose member data has aged out of `ListActive` (no
 *     `spawned_at` to order by);
 *   - older transcript history not yet paginated into `documentNodes`;
 *   - two dispatches spawned within the same millisecond (tie-guarded
 *     below — bails rather than trust an arbitrary tiebreak).
 *
 * Closed (reagent flagged this same residual gap across FOUR consecutive
 * review rounds on PR #2676 — the "believed rare" framing was wrong, and
 * so was this file's first attempted fix): two SAME-kind calls issued in
 * one turn (e.g. two parallel Agent-tool spawns) can have distinct,
 * non-tied `spawned_at` values whose relative order still doesn't match
 * transcript position — the kind-compatibility check can't catch this,
 * both sides read the same kind.
 *
 * The first attempted fix (a shared-`slug` batch check) was wrong: the
 * "one shared slug = one legitimate concurrent spawn" precedent
 * (REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md Finding 3) describes
 * MULTIPLE MEMBERS of one Task/Workflow-tool invocation sharing a batch
 * codename — not two SEPARATE solo Agent/Task tool_use calls issued in
 * parallel. Each solo call gets its own `solo_dispatch_id`, so two
 * parallel solo calls plausibly get two distinct, singleton slugs — the
 * guard never actually fired for the scenario it was meant to catch.
 *
 * What actually distinguishes "same turn" (ambiguous) from "different,
 * sequential turns" (safely orderable) is `ToolNode.timestamp` — but a
 * bare equality/near-tie check on it isn't the right shape (an earlier
 * draft of this comment dismissed it for exactly that reason). The right
 * shape is a THRESHOLD, because same-turn and different-turn gaps differ
 * by orders of magnitude for these specific tools: two tool_use blocks
 * inside ONE streamed assistant turn parse within single-digit
 * milliseconds of each other (no real-world wait between them — the
 * frontend is just iterating one already-arrived response). Two tool_use
 * blocks from DIFFERENT, sequential turns are separated by at least the
 * FIRST turn's own tool execution time, since the model can't start a next
 * turn before that turn's tool_result returns — and Agent/Task/Workflow
 * calls are inherently slow (real subagent work), so that gap is
 * realistically seconds-to-minutes, never sub-second. `SAME_TURN_THRESHOLD_MS`
 * sits comfortably between those two regimes.
 */

import type { AgentDispatch, ActiveSubagent } from "../../swarm/swarm-model";
import type { DocumentNode, ToolNode } from "../types";

const DISPATCH_TOOL_KINDS: ReadonlySet<ToolNode["tool"]> = new Set(["Agent", "Task", "Workflow"]);

/** See the module doc comment's closing paragraph. */
const SAME_TURN_THRESHOLD_MS = 2000;

function isDispatchToolNode(n: DocumentNode): n is ToolNode {
    return n.type === "tool" && DISPATCH_TOOL_KINDS.has((n as ToolNode).tool);
}

/** "Agent" and "Task" both produce a `kind: "solo"` AgentDispatch — treated
 *  as one category here since a parallel Agent+Task pair is exactly as
 *  ambiguous as a parallel Agent+Agent pair (both same-kind on the
 *  dispatch side). */
function dispatchCategory(tool: ToolNode["tool"]): "workflow" | "solo" {
    return tool === "Workflow" ? "workflow" : "solo";
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

    // Same-turn ambiguity guard (see the module doc comment above) — must
    // run before trusting ANY ordering, including the tie/kind checks
    // below, since a same-turn pair can have distinct, non-tied
    // `spawned_at` values and matching kinds yet still be unverifiably
    // ordered. Missing timestamps can't be ruled safe either — treated
    // the same as a too-close pair.
    const byCategory = new Map<"workflow" | "solo", ToolNode[]>();
    for (const n of toolNodes) {
        const cat = dispatchCategory(n.tool);
        const arr = byCategory.get(cat);
        if (arr) arr.push(n);
        else byCategory.set(cat, [n]);
    }
    for (const nodes of byCategory.values()) {
        for (let i = 0; i < nodes.length; i++) {
            for (let j = i + 1; j < nodes.length; j++) {
                const a = nodes[i].timestamp;
                const b = nodes[j].timestamp;
                if (a === undefined || b === undefined || Math.abs(a - b) < SAME_TURN_THRESHOLD_MS) {
                    return result;
                }
            }
        }
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

    // Tie guard (reagent/codex P1, PR #2676 review) — two dispatches
    // spawned within the same millisecond produce an unstable sort (falls
    // back to whichever order `dispatches`/`orderable` happened to arrive
    // in, unrelated to either transcript or true spawn order). This is
    // exactly the scenario that could silently swap TWO SAME-KIND calls
    // (e.g. two parallel Agent-tool spawns) even though the kind-
    // compatibility check below can't catch it — both sides read "Agent".
    // Bail on any tie rather than trust an arbitrary tiebreak.
    for (let i = 1; i < sorted.length; i++) {
        if (spawnOrderOf.get(sorted[i].dispatch_id) === spawnOrderOf.get(sorted[i - 1].dispatch_id)) {
            return result;
        }
    }

    // Kind-compatibility check (reagent P1, PR #2676 review) — count
    // equality alone doesn't guarantee a CORRECT pairing: if a turn spawns
    // an Agent/Task call and a Workflow call in parallel, and the
    // workflow's first member happens to sort earlier by spawned_at than
    // the task's own subagent, the counts still match but a zip-by-index
    // would swap kinds — pairing the Task's tool node with the Workflow
    // dispatch and vice versa, rendering the wrong card (member-count
    // progress on a solo call, or a solo "done" pill on a workflow).
    // Verify every pair's tool kind agrees with its dispatch's kind before
    // committing to ANY of them — one mismatch invalidates the whole pane,
    // same all-or-nothing policy as the count checks above.
    for (let i = 0; i < toolNodes.length; i++) {
        const expectsWorkflow = toolNodes[i].tool === "Workflow";
        const isWorkflow = sorted[i].kind === "workflow";
        if (expectsWorkflow !== isWorkflow) return result;
    }

    for (let i = 0; i < toolNodes.length; i++) {
        result.set(toolNodes[i].id, sorted[i]);
    }
    return result;
}
