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
 * rather than guessing.
 *
 * ## Same-kind ordering — history of this exact problem (read before touching)
 *
 * Two SAME-kind calls issued in one turn (e.g. two parallel Agent-tool
 * spawns) can have distinct, non-tied `spawned_at` values whose relative
 * order still doesn't match transcript position — the kind-compatibility
 * check below can't catch this, both sides read the same kind. Reagent
 * flagged this across FIVE consecutive review rounds on PR #2676, and TWO
 * different attempted heuristic fixes both turned out to be unsound:
 *
 * 1. A shared-`slug` batch check: wrong, because "one shared slug = one
 *    legitimate concurrent spawn" (REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md
 *    Finding 3) describes MULTIPLE MEMBERS of one Task/Workflow-tool
 *    invocation sharing a batch codename — not two SEPARATE solo Agent/Task
 *    calls issued in parallel. Each solo call gets its own
 *    `solo_dispatch_id` and plausibly its own distinct slug, so the guard
 *    never fired for the scenario it was meant to catch.
 * 2. A `ToolNode.timestamp`-gap threshold: also wrong. `timestamp` is the
 *    FRONTEND's own `Date.now()` at the moment each tool_call event is
 *    first parsed (`stream-parser.ts`) — not a fixed-latency signal. A
 *    single assistant turn can take many seconds to stream when the model
 *    generates a long description/prompt between two tool_use blocks
 *    (exactly what Agent/Task/Workflow calls carry), so two blocks
 *    genuinely in ONE turn can still land more than any chosen threshold
 *    apart — indistinguishable from two truly sequential, different-turn
 *    calls using timing alone, at any threshold.
 *
 * The only signal that's actually reliable is which assistant turn/message
 * each tool_use block came from — a structural fact, not something
 * inferrable from timing. That signal exists in the raw Anthropic
 * `message.id` one layer above this parser (`claude-translator.ts`'s
 * `handleMessageStart`), but isn't currently threaded through to
 * `ToolNode`, and adding it means touching `stream-parser.ts` — a
 * component this codebase's own comments flag as fragile, with a cited
 * history of regressions (PR #884/#885/#886, #1104, #1326) from exactly
 * this kind of "add a new field threaded through the live-streaming state
 * machine" change. Given two heuristic attempts have already both failed
 * review scrutiny, expanding into that component under review pressure is
 * a worse trade than accepting a coverage loss honestly.
 *
 * Final decision: bail whenever a pane has more than one dispatch-kind
 * tool node of the SAME category (`solo` — covers `Agent`/`Task` — or
 * `workflow`) that would need relative ordering. This is provably correct
 * — no ordering claim is ever made among same-category calls, so there's
 * nothing to get wrong — at the cost of matching only when a pane has at
 * most one live Agent/Task-kind call and at most one Workflow-kind call.
 * Sequential (non-parallel) subagent calls across a session are NOT
 * affected differently than before — a pane's dispatch-kind tool nodes
 * are the calls STILL LIVE/relevant at correlation time, not the session's
 * entire history — but a pane with two GENUINELY concurrent, still-live
 * calls of the same kind falls back to `CompactResult` for both instead of
 * risking a wrong pairing. Threading a real per-turn identity through
 * `stream-parser.ts` would close this gap without the coverage cost, and
 * is a legitimate follow-up, not required to ship this feature safely.
 */

import type { AgentDispatch, ActiveSubagent } from "../../swarm/swarm-model";
import type { DocumentNode, ToolNode } from "../types";

const DISPATCH_TOOL_KINDS: ReadonlySet<ToolNode["tool"]> = new Set(["Agent", "Task", "Workflow"]);

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

    // Same-category cap (see the module doc comment above) — no ordering
    // claim is ever made among two-or-more same-category dispatch-kind
    // tool nodes, since their relative order can't be verified. Must run
    // before trusting ANY ordering below.
    const categoryCounts = new Map<"workflow" | "solo", number>();
    for (const n of toolNodes) {
        const cat = dispatchCategory(n.tool);
        categoryCounts.set(cat, (categoryCounts.get(cat) ?? 0) + 1);
    }
    for (const count of categoryCounts.values()) {
        if (count > 1) return result;
    }

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
    //  - counts disagree with the transcript's tool-node count — a
    //    dispatch pruned entirely from ListDispatches, or unpaginated
    //    older history, shows up here as a mismatch, which is the safe
    //    direction to fail in (undercount -> no match, never a wrong
    //    match).
    if (blockDispatches.length !== orderable.length) return result;
    if (toolNodes.length !== orderable.length) return result;

    // No same-category ties are possible past the cap above (at most one
    // dispatch per category survives it), so a plain spawn-order sort is
    // safe here — any residual instability could only occur BETWEEN
    // categories, which the kind-compatibility check below independently
    // guards against regardless of sort outcome.
    const sorted = [...orderable].sort((a, b) => spawnOrderOf.get(a.dispatch_id)! - spawnOrderOf.get(b.dispatch_id)!);

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
