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
 * Closed (reagent flagged this same residual gap across three consecutive
 * review rounds on PR #2676 — the prior "believed rare" framing wasn't a
 * sufficient answer, and parallel same-turn spawns are actually a common
 * usage pattern, not an edge case): two SAME-kind calls issued in one turn
 * (e.g. two parallel Agent-tool spawns) can have distinct, non-tied
 * `spawned_at` values whose relative order still doesn't match transcript
 * position — the kind-compatibility check can't catch this, both sides
 * read the same kind, and even a `ToolNode.timestamp` comparison isn't a
 * reliable secondary signal (it's the FRONTEND's own receive-time during
 * SSE parsing, not the CLI's dispatch time — two truly parallel tool_use
 * blocks can still land microseconds apart there). The one signal that
 * actually IS documented and reliable in this codebase: Claude Code's own
 * `slug` is a per-CONCURRENT-BATCH codename — "one shared slug = one
 * legitimate concurrent spawn" (REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md
 * Finding 3). Two solo dispatches sharing a slug (or both missing one) are
 * from the same batch and their relative order is unverifiable — bail.
 * Distinct slugs mean distinct batches/turns, safely orderable by position
 * (a turn's own tool_uses always resolve before the next turn's begin).
 * Workflow dispatches have no equivalent per-run batch signal, so more
 * than one Workflow-kind dispatch in a single pane's match always bails —
 * a real, accepted coverage loss for that specific (rarer) case, not a
 * silent gap.
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

    // Same-batch ambiguity guard (see the module doc comment above) — must
    // run before trusting ANY ordering, including the tie/kind checks
    // below, since a same-batch pair can have distinct spawned_at values
    // and matching kinds yet still be unverifiably ordered.
    const NO_SLUG = "\0no-slug";
    const soloSlugCounts = new Map<string, number>();
    for (const d of orderable) {
        if (d.kind !== "solo") continue;
        const slug = subagents.find((s) => s.dispatch_id === d.dispatch_id)?.slug || NO_SLUG;
        soloSlugCounts.set(slug, (soloSlugCounts.get(slug) ?? 0) + 1);
    }
    for (const count of soloSlugCounts.values()) {
        if (count > 1) return result;
    }
    const workflowCount = orderable.filter((d) => d.kind === "workflow").length;
    if (workflowCount > 1) return result;

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
