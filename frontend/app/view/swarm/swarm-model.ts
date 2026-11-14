// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi, type BackgroundTaskView, type FleetActionResult, type FleetStagePlan } from "@/app/store/rpc-api";
import {
    localFleetTargets,
    EMPTY_FLEET_RESULT,
    labelsOf,
    partitionTargets,
    remoteFleetTargets,
    withUnreachable,
    type FleetAction,
    type FleetResultEntry,
    type FleetTargetInfo,
} from "./swarm-fleet-targets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { MOS } from "@/app/store/global";
import { callBackendService } from "@/store/mos";
import { BlockService } from "@/app/store/services";
import { readSwarmSummary } from "@/app/store/activitySummary";
import { contextReadingFromMeta } from "@/app/store/agent-pane-state/context-reading";
import { resolveSwarmLine, type SwarmLine } from "@/app/store/swarm-line";
import type { SupervisorMode } from "@/types/rpc/SupervisorMode";
import { remoteSections, type SwarmOtherInstances } from "./swarm-remote";
import { actionableKeys } from "./swarm-filter-state";
import { createSignal, untrack, type Accessor, type Setter } from "solid-js";
import { sameList, traced } from "@/app/util/select-trace";
import { countdownCandidates, withRetiredEntry } from "./swarm-retire";
import { groupBackgroundTasks, type AgentBackgroundTasks } from "./swarm-background";
import { toolDetail } from "@/app/agent-model/tool-descriptors";
import { onSubagentNamed, withSubagentName } from "./subagent-naming";
import { withAgentProgress, withAgentStatus } from "./swarm-updates";
import { refetchAbsentWindowBlocks } from "./swarm-block-heal";

// ── Types ────────────────────────────────────────────────────────────────

export type { ActiveSubagent } from "@/app/agent-model/subagent-types";
import type { ActiveSubagent } from "@/app/agent-model/subagent-types";

/**
 * Mirrors the backend's `ShellSummary` (`shell_node.rs`): one RUNNING background shell the agent
 * started via the Shell MCP tool, from `shell.ListActive` (unfiltered, like `ActiveSubagent`);
 * `buildTree()` groups by `block_id`. Exited shells never appear: Phase 1 shows what is happening
 * now, not history (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20).
 */
export interface ActiveShell {
    shell_id: string;
    block_id: string;
    cmd: string;
    /** Caller-supplied display title, defaulting to `cmd` server-side. */
    title: string;
    started_at: number;
    line_count: number;
}

/**
 * Mirrors the backend's `CronSummary` (`server/cron.rs`): a persistent cron job whose
 * `created_by` agent resolves to a live block, from `cron.ListActive`, grouped by `block_id`
 * in `buildTree()`. Shown enabled or paused (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2).
 */
export interface ActiveCron {
    id: string;
    block_id: string;
    name: string;
    expression: string;
    target: string;
    created_by: string;
    enabled: boolean;
    last_fired: number | null;
    fire_count: number;
    max_fires: number | null;
    next_fire: string | null;
    /** Scheduled fires that reached no one, and why the last one did (`target_stopped`, ...). */
    missed_count?: number;
    last_miss_reason?: string;
}

/**
 * Mirrors the backend's `AgentDispatch` (one per Agent-tool or Workflow-tool call). Only
 * Workflow-kind dispatches get their own tree row (`WorkflowDispatch`); a Solo dispatch's one
 * member renders as a plain `ActiveSubagent` row, no wrapper (SPEC §5/§7).
 */
export interface AgentDispatch {
    dispatch_id: string;
    kind: "solo" | "workflow";
    parent_agent: string;
    parent_block_id: string;
    session_id: string;
    member_count: number;
    members_done: number;
    /**
     * `"abandoned"` (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2): every
     * member is Completed|Abandoned and at least one Abandoned (the parent block's turn ended
     * first). Both `"completed"` and `"abandoned"` are terminal: `WorkflowDispatch.status` folds
     * them into `"retired"`.
     */
    status: "running" | "completed" | "abandoned";
    last_event_at: number;
    /**
     * A Workflow-kind dispatch's concise Haiku-generated name, resolved EAGERLY the first time its
     * first member is observed live (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A),
     * not on click like `ActiveSubagent.display_name`; `null` until resolved.
     *
     * A Solo-kind dispatch mirrors its one member's `display_name` (`subagent_watcher/query.rs`'s
     * `solo_dispatch()`), so it is `null` until that resolves.
     */
    dispatch_name: string | null;
}

// ── Subagent event log (inline-expand detail) ───────────────────────────

export interface SubagentEvent {
    agent_id: string;
    event_type: SubagentEventType;
    timestamp: number;
}

type SubagentEventType =
    | { type: "text"; content: string }
    /** `input`: the call's input, each field cut by the backend
     *  (`subagent_watcher/parse.rs`'s `bounded_tool_input`). */
    | { type: "tool_use"; name: string; input: unknown }
    | { type: "tool_result"; is_error: boolean; preview: string }
    | { type: "progress"; output: string }
    | { type: "result"; content: string };

/**
 * A subagent tool call's detail line: the agent pane's own header detail
 * (`toolDetail`), so both show the same thing for the same call. A tool with
 * no detail shows its input as compact JSON instead, since this feed has
 * nothing else to show for it.
 */
export function subagentToolDetail(name: string, input: unknown): string {
    if (input == null) return "";
    const params = typeof input === "object" && !Array.isArray(input) ? (input as Record<string, any>) : undefined;
    const detail = toolDetail(name, params);
    return typeof detail === "string" && detail ? detail : JSON.stringify(input);
}

/**
 * One row for a Workflow-kind `AgentDispatch` (SPEC §7): never one row per member, however many
 * (one run spawned 1,030+; docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md). It does NOT
 * carry the member list: a dispatch this large can't hold its members' event histories in the tree
 * atom without reintroducing the unbounded volume that retro found. Expanding the row shows a live
 * concatenated activity feed (`createDispatchDetail`), not nested member rows.
 */
export interface WorkflowDispatch {
    kind: "workflowDispatch";
    dispatchId: string;
    /**
     * Prefers the backend's eagerly-resolved `AgentDispatch.dispatch_name`
     * (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A); falls back to a member's
     * `slug`, then the raw `dispatchId`, for the window before the eager Haiku call resolves (or if
     * it never does).
     */
    name: string;
    memberCount: number;
    membersDone: number;
    /**
     * "active" if any member is still active; "retired" once every member has completed or been
     * abandoned (both non-"running" `AgentDispatch.status` values fold into it).
     */
    status: "active" | "retired";
    lastEventAt: number;
}

export interface AgentTreeNode {
    blockId: string | null;
    agentName: string;
    agentProvider: string | null;
    activitySummary: string | null;
    /** What the row shows: the generated title, or the first fallback that has
     *  something (never empty). `activitySummary` stays the generated title
     *  alone, so the view can flash on a new one. See store/swarm-line.ts. */
    line: SwarmLine;
    contextTokens: number | null;
    agentStatus: "running" | "idle";
    /**
     * One row per Agent-tool (solo) dispatch: always a flat list, never grouped by shared name
     * (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §2, superseding #2226's `NameGroup`:
     * eager per-dispatch naming removed the "many rows share one label" problem). Sorted by most
     * recent activity.
     */
    agentToolRows: ActiveSubagent[];
    /**
     * One row per Workflow-tool dispatch, always its own row (SPEC §7). Sorted by most recent
     * activity.
     */
    workflowRows: WorkflowDispatch[];
    /**
     * One row per currently-RUNNING background shell this agent started
     * (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 1), newest first.
     */
    shellRows: ActiveShell[];
    /**
     * One row per cron job this agent created (same spec, Phase 2), enabled or paused; newest
     * `last_fired` first, never-fired last.
     */
    cronRows: ActiveCron[];
    /**
     * The agent's current todo checklist in its own order (NOT sorted: the order it wrote is the
     * order it means). Pushed by the backend's `agent:progress` sweep, not read from block meta,
     * because the frontend holds tool calls only for mounted panes and the Swarm exists for the
     * ones that aren't.
     */
    todoRows: TodoItem[];
    /** How many rows `MAX_TODOS` dropped backend-side, so the tree can say
     *  "+N more" instead of showing a partial list as if it were whole. */
    todosTruncated: number;
    /**
     * True when the backend first saw this block mid-stream, so item-at-a-time (`TaskCreate`)
     * entries from before were never observed. Not folded into `todosTruncated`: that is a cap we
     * chose and can count, this is history we could not see, and rendering them alike would claim a
     * precision we lack.
     */
    todosPartial: boolean;
    /** The tool this agent is running right now, or null between tools.
     *  Complements `activitySummary`: that is a Haiku paraphrase of the last
     *  ~20s, this is the literal call in flight. */
    currentTool: string | null;
    /** This agent's background tasks from the durable registry, split into
     *  the ones each solo subagent row owns and the rest (see
     *  swarm-background.ts). Unfiltered by time: the view applies the
     *  finished-row retention on its own clock. */
    backgroundTasks: AgentBackgroundTasks;
}

/** One checklist entry, as published by the backend progress watcher. */
export interface TodoItem {
    text: string;
    /**
     * `pending` | `in_progress` | `completed`, or whatever a provider wrote: passed through
     * verbatim so an unrecognized status renders as itself, not as "pending".
     */
    status: string;
}

/** One `agent:progress` payload, keyed by block. */
export interface AgentProgress {
    todos: TodoItem[];
    todosTruncated: number;
    todosPartial: boolean;
    currentTool: string | null;
}

/**
 * Builds one block's two fixed row buckets from its (block-filtered) `AgentDispatch`es and raw
 * `SubAgent`s: always these two groupings, never data-driven collapsing by shared name (see
 * `AgentTreeNode.agentToolRows`). Each is sorted by recency.
 */
export function buildDispatchBuckets(
    dispatches: AgentDispatch[],
    subagents: ActiveSubagent[]
): { agentToolRows: ActiveSubagent[]; workflowRows: WorkflowDispatch[] } {
    const workflowRows: WorkflowDispatch[] = dispatches
        .filter((d) => d.kind === "workflow")
        .map((d) => {
            const namedMember = subagents.find((s) => s.dispatch_id === d.dispatch_id && s.slug);
            return {
                kind: "workflowDispatch" as const,
                dispatchId: d.dispatch_id,
                name: d.dispatch_name || namedMember?.slug || d.dispatch_id,
                memberCount: d.member_count,
                membersDone: d.members_done,
                // Both "completed" and "abandoned" are terminal: an abandoned dispatch (dead
                // parent, never finished) must not read as "active".
                status: d.status === "running" ? ("active" as const) : ("retired" as const),
                lastEventAt: d.last_event_at,
            };
        })
        .sort((a, b) => b.lastEventAt - a.lastEventAt);

    const solo = subagents.filter((s) => s.dispatch_id.startsWith("solo:"));

    // Fallback for a failed or lagging `ListDispatches`: `loadDispatches()` swallows RPC errors and
    // leaves `dispatchesAtom` stale, so `dispatches` can miss entries that `subagents` (a separate
    // fetch) already has, and a workflow-kind subagent with no `workflowRows` entry would vanish
    // from the tree. Each orphaned `dispatch_id` becomes ONE placeholder `WorkflowDispatch` row
    // appended to `workflowRows`, never spread into `agentToolRows`: that turned a lagging
    // dispatch's members into N Agent-Tool rows (up to member_count), breaking the
    // one-row-per-Workflow-call invariant
    // (docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.1).
    const workflowDispatchIds = new Set(workflowRows.map((w) => w.dispatchId));
    const orphanedWorkflowMembers = subagents.filter(
        (s) => !s.dispatch_id.startsWith("solo:") && !workflowDispatchIds.has(s.dispatch_id)
    );
    const orphanedByDispatchId = new Map<string, ActiveSubagent[]>();
    for (const member of orphanedWorkflowMembers) {
        const group = orphanedByDispatchId.get(member.dispatch_id);
        if (group) group.push(member);
        else orphanedByDispatchId.set(member.dispatch_id, [member]);
    }
    const placeholderWorkflowRows: WorkflowDispatch[] = Array.from(
        orphanedByDispatchId.entries()
    ).map(([dispatchId, members]) => {
        const named = members.find((m) => m.display_name) ?? members.find((m) => m.slug);
        return {
            kind: "workflowDispatch" as const,
            dispatchId,
            name: named?.display_name || named?.slug || dispatchId,
            // Best-effort lower bound: the real member_count is unknown until ListDispatches
            // catches up; this counts only the members ListActive has surfaced.
            memberCount: members.length,
            membersDone: members.filter((m) => m.status !== "active").length,
            status: members.some((m) => m.status === "active") ? ("active" as const) : ("retired" as const),
            lastEventAt: Math.max(...members.map((m) => m.last_event_at)),
        };
    });

    const agentToolRows = solo.sort((a, b) => b.last_event_at - a.last_event_at);
    const allWorkflowRows = [...workflowRows, ...placeholderWorkflowRows].sort(
        (a, b) => b.lastEventAt - a.lastEventAt
    );

    return { agentToolRows, workflowRows: allWorkflowRows };
}

/**
 * Builds one block's Shell bucket rows (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 1):
 * every running shell with a matching `block_id`, newest first. A pure function, like
 * `buildDispatchBuckets`, so it is unit-testable without `SwarmViewModel`.
 */
export function buildShellRows(shells: ActiveShell[], blockId: string | null): ActiveShell[] {
    return shells
        .filter((s) => s.block_id === blockId)
        .sort((a, b) => b.started_at - a.started_at);
}

/**
 * Builds one block's Cron bucket rows (same spec, Phase 2): every cron job with a matching
 * `block_id`, newest last-fired first; a never-fired job sorts last (no recency signal). Pure, for
 * the same reason as `buildShellRows`.
 */
export function buildCronRows(crons: ActiveCron[], blockId: string | null): ActiveCron[] {
    return crons
        .filter((c) => c.block_id === blockId)
        .sort((a, b) => (b.last_fired ?? -1) - (a.last_fired ?? -1));
}

/**
 * The context size a Swarm row shows: the agent pane's own validated reading,
 * mirrored to `agent:context` (store/agent-pane-state/context-reading.ts). The
 * legacy bare `term:ctx-tokens` is ignored: older builds wrote turn totals
 * there (a "17m" context), and a pane that isn't open never rewrites it.
 */
export function swarmContextTokens(meta: MetaType | null | undefined): number | null {
    return contextReadingFromMeta(meta?.["agent:context"])?.tokens ?? null;
}

/**
 * Whether a blockId in `buildTree()`'s row list should render a row: false only when its MOS oref
 * has definitively resolved to nothing. While loading it returns `true` (render, as before;
 * self-corrects when the fetch resolves) because `MOS.getMuxObjectAtom` seeds a fresh oref with `{
 * value: null, loading: true }` (`mos.ts`'s `createMuxValueObject`), so a just-spawned real block
 * would otherwise look like a phantom on the first `buildTree()` pass (#2438; plausibly the cause
 * of "a live-spawned subagent wasn't observed in Swarm").
 *
 * A `null` block with `isLoading === false` means the fetch completed with nothing there: usually a
 * subagent's `parent_block_id` (added to the row-id set as a registration-ordering fallback) that
 * never resolved to a registered block, or a pruned block whose subagent record outlived it
 * (`prune_block` runs on BlockDeleted/TabDeleted/WorkspaceDeleted; a block whose own registration
 * never completed was never "deleted"). A placeholder `"Agent"` row for such an id is
 * indistinguishable from a real agent and leaves any grouped dispatch as an empty "No activity yet"
 * row. Distinct from a real block whose `agentName` hasn't reached its meta yet (`buildTree()`'s
 * `"Agent"` fallback). Pure, so it is unit-testable without MOS
 * (RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06.md).
 */
export function hasRenderableBlock<T>(block: T | null | undefined, isLoading: boolean): boolean {
    if (isLoading) return true;
    return block != null;
}

export type { FleetResultEntry } from "./swarm-fleet-targets";

/**
 * Rows eligible for the bulk "Clear completed" action: every terminal-status (not "active") row
 * across all `AgentTreeNode`s as `(rowKey, lastEventAt)` pairs for `retireRow`. Pure, for unit
 * tests. Already-retired rows need no exclusion: `buildTree()` filters them out (`filterRetired`)
 * before this runs (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3).
 */
export function collectClearableRows(nodes: AgentTreeNode[]): { rowKey: string; lastEventAt: number }[] {
    const result: { rowKey: string; lastEventAt: number }[] = [];
    for (const node of nodes) {
        for (const s of node.agentToolRows) {
            if (s.status !== "active") {
                result.push({ rowKey: subagentRowKey(s.agent_id), lastEventAt: s.last_event_at });
            }
        }
        for (const w of node.workflowRows) {
            // `WorkflowDispatch.status` "retired" means every member completed (backend-derived),
            // NOT that the user retired the row: a naming collision with this feature, not a bug.
            if (w.status !== "active") {
                // `workflowRetireSignal`, not `w.lastEventAt` (#2677).
                result.push({ rowKey: w.dispatchId, lastEventAt: workflowRetireSignal(w) });
            }
        }
    }
    return result;
}

/**
 * The label `SubagentRow` shows, in priority: `display_name` (Haiku-resolved, eagerly at dispatch
 * time for the common case; SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A) > `slug` +
 * a short `agent_id` suffix > the short `agent_id`.
 *
 * `slug` is NOT unique per subagent: it is read from the first line of the subagent's JSONL
 * (`subagent_watcher/parse.rs`'s `read_jsonl_from_offset`), the CLI's per-session/per-batch
 * codename, so a whole Task/Workflow batch of distinct subagents legitimately shares one
 * (`WorkflowDispatch.name` relies on that as a fallback). A bare `slug` as a row label rendered
 * every member of such a batch identically until its `display_name` resolved (task #44: 17 distinct
 * `agent_id`s sharing one slug rendered as 17 apparent duplicates; not a slug or spawn-dedup bug).
 * The always-unique `agent_id.substring(0, 7)` suffix, the same disambiguator the retired
 * `SubagentDetailPane` showed as a meta chip, keeps siblings distinct without claiming a uniqueness
 * `slug` lacks.
 */
export function subagentDisplayLabel(
    sub: Pick<ActiveSubagent, "display_name" | "slug" | "agent_id">
): string {
    if (sub.display_name) return sub.display_name;
    const shortId = sub.agent_id.substring(0, 7);
    return sub.slug ? `${sub.slug} · ${shortId}` : shortId;
}

function shallowEqualSubagent(a: ActiveSubagent, b: ActiveSubagent): boolean {
    return (
        a.slug === b.slug &&
        a.status === b.status &&
        a.last_event_at === b.last_event_at &&
        a.event_count === b.event_count &&
        a.model === b.model &&
        a.dispatch_id === b.dispatch_id &&
        a.display_name === b.display_name &&
        a.parent_agent === b.parent_agent &&
        a.parent_block_id === b.parent_block_id &&
        a.session_id === b.session_id
    );
}

/**
 * Merges a freshly-fetched subagent list into the previous one, reusing the OLD object reference
 * for entries whose fields are unchanged. `ListActive` returns a new array every call; without this
 * every subagent gets a new identity on each spawn/completed refresh, and SolidJS's `<For>` (which
 * diffs by reference) remounts every row, collapsing any the user expanded.
 */
export function mergeSubagentsPreservingIdentity(
    prev: ActiveSubagent[],
    next: ActiveSubagent[]
): ActiveSubagent[] {
    const prevById = new Map(prev.map((s) => [s.agent_id, s]));
    return next.map((incoming) => {
        const old = prevById.get(incoming.agent_id);
        return old && shallowEqualSubagent(old, incoming) ? old : incoming;
    });
}

/**
 * Content equality for a `WorkflowDispatch`, used by `stabilizeGroupIdentity` to reuse the old
 * reference or accept the new one.
 */
function shallowEqualWorkflowDispatch(a: WorkflowDispatch, b: WorkflowDispatch): boolean {
    return (
        a.dispatchId === b.dispatchId &&
        a.name === b.name &&
        a.memberCount === b.memberCount &&
        a.membersDone === b.membersDone &&
        a.status === b.status &&
        a.lastEventAt === b.lastEventAt
    );
}

/**
 * Cache key for a `WorkflowDispatch`: `wf:`-namespaced although only one kind uses the cache now,
 * so `expandedIdsAtom`'s ids stay stable across the two-bucket redesign
 * (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19).
 */
export function groupCacheKey(child: WorkflowDispatch): string {
    return `wf:${child.dispatchId}`;
}

/**
 * Row identity for an Agent Tool row: the `agent:${agent_id}` key `swarm-view.tsx`'s `SubagentRow`
 * computes for expand state and `DispatchActivityFeed` identity
 * (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4, decoupled from `dispatch_id` in #2232;
 * see `getDispatchDetail`). Exported so the retire filter and the Retire button compute one key,
 * not two drifting templates.
 */
export function subagentRowKey(agentId: string): string {
    return `agent:${agentId}`;
}

/**
 * The "has anything genuinely new happened" signal `filterRetired` compares for a
 * `WorkflowDispatch` row: `memberCount` / `membersDone`, not `lastEventAt` (#2677). `lastEventAt`
 * comes from two different clocks: a synthesized placeholder row (`buildDispatchBuckets`'s
 * `orphanedWorkflowMembers` fallback) takes the max of each member's own event timestamp (when
 * Claude Code wrote the JSONL line), while the real `AgentDispatch` (`refresh_dispatch_info` in
 * agentmux-srv) sets `now_millis()`, when the backend processed the event. The two never match for
 * the same activity, so a placeholder retired under one clock looked like new activity once
 * `ListDispatches` caught up, un-retiring exactly the row the fallback and the persisted retire
 * exist to keep dismissed. The counts converge to the same numbers for the same member set,
 * whichever representation renders it.
 */
export function workflowRetireSignal(w: Pick<WorkflowDispatch, "memberCount" | "membersDone">): number {
    return w.membersDone * 1_000_000 + w.memberCount;
}

/**
 * Drop any row whose key is in `retired` and whose `lastEventAt` still matches the snapshot taken
 * at retire time; newer activity for the same key shows the row again with no separate un-retire
 * action (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6). Shared by both
 * `AgentTreeNode` buckets in `buildTree()`, generic over `T`.
 */
export function filterRetired<T>(
    rows: T[],
    retired: Map<string, number>,
    rowKey: (row: T) => string,
    lastEventAt: (row: T) => number
): T[] {
    return rows.filter((row) => retired.get(rowKey(row)) !== lastEventAt(row));
}

/**
 * Drop `retired` entries whose key is no longer in `liveKeys` at all: a row gone from live state
 * (block closed, dispatch pruned, #2233) can never un-retire itself, so its entry would only grow
 * for the life of the session (#2235). A row still live keeps its entry. Returns `retired`
 * unchanged (same reference) when nothing was dropped, so a signal setter doesn't fire a no-op
 * update.
 */
export function pruneRetiredEntries(retired: Map<string, number>, liveKeys: Set<string>): Map<string, number> {
    let changed = false;
    const next = new Map<string, number>();
    for (const [key, lastEventAt] of retired) {
        if (liveKeys.has(key)) {
            next.set(key, lastEventAt);
        } else {
            changed = true;
        }
    }
    return changed ? next : retired;
}

// Retired-row persistence (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3).
//
// `_retiredRowKeys` was in-memory only, so a dismissed dead-session row came back on every reload,
// a direct contributor to the "48 rows for a couple of real calls" symptom that spec root-causes.
// It is now persisted to localStorage, local-machine scope (not synced; a deliberate choice, spec
// §6), following `toolchain-view.tsx`'s `loadWidgetPorts` / `saveWidgetPort`: a namespaced key,
// JSON, and a defensive try/catch because localStorage can throw in some embedding contexts.

const RETIRED_ROW_KEYS_STORAGE_KEY = "agentmux:swarm-retired-rows";

export function loadRetiredRowKeysFromStorage(): Map<string, number> {
    try {
        const raw = localStorage.getItem(RETIRED_ROW_KEYS_STORAGE_KEY);
        if (!raw) return new Map();
        const entries = JSON.parse(raw) as [string, number][];
        return new Map(entries);
    } catch {
        return new Map();
    }
}

export function saveRetiredRowKeysToStorage(retired: Map<string, number>): void {
    try {
        localStorage.setItem(RETIRED_ROW_KEYS_STORAGE_KEY, JSON.stringify(Array.from(retired.entries())));
    } catch {
        // best-effort: a full localStorage, or a context where it throws, just means this session's
        // retires don't survive a reload.
    }
}

/**
 * Stabilize `WorkflowDispatch` wrapper identity across `buildTree()` calls, mirroring
 * `mergeSubagentsPreservingIdentity` one level up. `buildDispatchBuckets` is pure and builds new
 * wrapper objects on every call, and a fresh wrapper remounts `WorkflowDispatchRow` (and everything
 * nested in an expanded one) on every unrelated tree recompute, defeating the remount fix
 * `expandedIdsAtom` / `getDispatchDetail` provide for a large dispatch.
 *
 * `cache` is a `Map<groupCacheKey, dispatch>` the caller keeps (one per `SwarmViewModel`), shared
 * by the once-per-BLOCK calls of one `buildTree()` pass. This only reuses or replaces entries and
 * never prunes: pruning scoped to one block's rows would evict entries another block's call just
 * wrote. After a full multi-block rebuild, prune once with `pruneGroupIdentityCache`.
 */
export function stabilizeGroupIdentity(
    cache: Map<string, WorkflowDispatch>,
    rows: WorkflowDispatch[]
): WorkflowDispatch[] {
    return rows.map((row) => {
        const key = groupCacheKey(row);
        const old = cache.get(key);
        const stable = old && shallowEqualWorkflowDispatch(old, row) ? old : row;
        cache.set(key, stable);
        return stable;
    });
}

/** Drop cache entries for dispatches no longer present anywhere in the tree
 *  — call once after a full `buildTree()` pass, not per-block (see
 *  `stabilizeGroupIdentity`), so it can't grow unbounded across sessions. */
export function pruneGroupIdentityCache(cache: Map<string, WorkflowDispatch>, liveGroupKeys: Set<string>): void {
    for (const key of [...cache.keys()]) {
        if (!liveGroupKeys.has(key)) cache.delete(key);
    }
}

// ── Dispatch concatenated activity feed (SPEC §7, generalized to solo
//    dispatches in SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4) ──

export interface DispatchActivityEntry {
    agentId: string;
    event: SubagentEvent;
}

export interface DispatchDetail {
    entriesAtom: Accessor<DispatchActivityEntry[]>;
    dispose: () => void;
}

/** Hard cap on the concatenated feed's retained entries — a Workflow
 *  dispatch with hundreds of members can generate activity fast enough that
 *  an unbounded feed would itself become the same unbounded-growth problem
 *  this redesign exists to avoid. Oldest entries drop first. */
const MAX_DISPATCH_FEED_ENTRIES = 500;

/** How long a clean-terminal row lingers before auto-retiring
 *  (SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06). Exported so
 *  swarm-view.tsx's countdown display computes remaining time against the
 *  same constant the ViewModel's own timer actually runs on, rather than a
 *  second hardcoded `60_000` that could drift out of sync. */
export const AUTO_RETIRE_DELAY_MS = 60_000;

/** One row's auto-linger countdown state — see `SwarmViewModel.
 *  _countdownState`'s doc comment for the `pausedAt` freeze mechanism. */
export interface CountdownEntry {
    lastEventAt: number;
    startedAt: number;
    pausedAt: number | null;
}

/** Identifies one `DispatchActivityEntry` for de-dup purposes — an
 *  (agentId, timestamp, event payload) triple is stable across both the
 *  live `dispatch:activity` broadcast and a `GetHistory` backfill, since
 *  both ultimately read the same underlying event, not a re-generated one. */
function dispatchActivityEntryKey(e: DispatchActivityEntry): string {
    return `${e.agentId}:${e.event.timestamp}:${JSON.stringify(e.event.event_type)}`;
}

/**
 * Merge freshly arrived entries into `prev`: de-duplicate by `dispatchActivityEntryKey`, sort by
 * timestamp, cap to `MAX_DISPATCH_FEED_ENTRIES`. Exported (pure, no signal access) so the de-dup is
 * unit-testable (swarm-model.test.ts).
 *
 * De-dup matters because a solo dispatch's `GetHistory` backfill and the live `dispatch:activity`
 * broadcast race: an event the backend flushed into `pending_activity` before `GetHistory` resolves
 * is in both, so concatenating would double-count it (#2232).
 */
export function mergeDispatchActivityEntries(
    prev: DispatchActivityEntry[],
    incoming: DispatchActivityEntry[]
): DispatchActivityEntry[] {
    if (incoming.length === 0) return prev;
    const seen = new Set(prev.map(dispatchActivityEntryKey));
    const fresh = incoming.filter((e) => {
        const key = dispatchActivityEntryKey(e);
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
    });
    if (fresh.length === 0) return prev;
    const merged = [...prev, ...fresh].sort((a, b) => a.event.timestamp - b.event.timestamp);
    return merged.length > MAX_DISPATCH_FEED_ENTRIES
        ? merged.slice(merged.length - MAX_DISPATCH_FEED_ENTRIES)
        : merged;
}

/**
 * Expanding a row (`WorkflowDispatchRow`, or an Agent Tool row,
 * SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4) shows this: every member's new events
 * merged into one chronological, member-tagged stream, fed by the backend's coalesced
 * `dispatch:activity` broadcast (`flush_pending_dispatch_activity` in `subagent_watcher/jsonl.rs`;
 * solo `dispatch_id`s too since Phase A, dual-emitted with the immediate `subagent:activity`).
 *
 * Live activity is live-only for a Workflow dispatch: SPEC §9.4 leaves backfill for a
 * multi-hundred-member workflow open (fetching thousands of prior events on every expand would
 * bring back the request and render volume this redesign removes), and no bulk-backfill RPC exists.
 * A single subagent's history is bounded and `subagent.GetHistory` exists, so it is backfilled
 * here, as the retired `SubagentDetailPane` / `createSubagentDetail` did.
 *
 * `backfillAgentId` (optional) scopes both the `GetHistory` backfill and the live
 * `dispatch:activity` subscription to one agent's events. A single-subagent row (any Agent Tool
 * row, including an orphaned workflow member, see `getDispatchDetail`) should pass the subagent's
 * own `agent_id` rather than rely on a `"solo:"` `dispatchId` prefix: the prefix doesn't cover an
 * orphaned member, whose `dispatchId` can be shared with sibling rows and would show their live
 * events. Without it, the `"solo:"` prefix is parsed, for callers with no `ActiveSubagent`. A
 * multi-member `WorkflowDispatchRow` omits it and gets every member's events.
 */
export function createDispatchDetail(dispatchId: string, backfillAgentId?: string): DispatchDetail {
    const [entries, setEntries] = createSignal<DispatchActivityEntry[]>([]);

    const mergeIncoming = (incoming: DispatchActivityEntry[]): void => {
        if (incoming.length === 0) return;
        setEntries((prev) => mergeDispatchActivityEntries(prev, incoming));
    };

    const unsub = onMuxEvent(WpsEvent.DispatchActivity, (data) => {
            if (data.dispatchId !== dispatchId) return;
            const members = data.members;
            const relevant = backfillAgentId ? members.filter((m) => m.agentId === backfillAgentId) : members;
            mergeIncoming(relevant.flatMap((m) => (m.events ?? []).map((evt) => ({ agentId: m.agentId, event: evt }))));
        });

    const soloAgentId = backfillAgentId ?? (dispatchId.startsWith("solo:") ? dispatchId.slice("solo:".length) : undefined);
    if (soloAgentId) {
        const agentId = soloAgentId;
        void (async () => {
            try {
                const result = await callBackendService("subagent", "GetHistory", [agentId, MAX_DISPATCH_FEED_ENTRIES]);
                const events = (result as SubagentEvent[]) ?? [];
                mergeIncoming(events.map((event) => ({ agentId, event })));
            } catch {
                // ignore — feed still works live-only
            }
        })();
    }

    return {
        entriesAtom: entries,
        dispose: () => {
            unsub?.();
        },
    };
}

// ── Status derivation ────────────────────────────────────────────────────

/**
 * `turn_active` is turn-precise (from the health monitor on the NDJSON stream) but only meaningful
 * for persistent/ACP agent controllers. `is_agent_pane` is the discriminator, because the Rust
 * struct omits `false` fields (`skip_serializing_if = "is_false"`), so `turn_active: false` and
 * "never populated" look the same on the wire. For `is_agent_pane` panes trust `turn_active` alone:
 * `shellprocstatus` stays `"running"` for a persistent agent's whole process lifetime, idle
 * included, so OR-ing it in would show an idle agent as working. Other panes (shell/PTY, no turn
 * concept) fall back to `shellprocstatus`. See
 * docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md Finding 1.
 */
function derivedRunningStatus(
    isAgentPane: boolean | undefined,
    turnActive: boolean | undefined,
    shellprocstatus: string | undefined,
): "running" | "idle" {
    if (isAgentPane) return turnActive ? "running" : "idle";
    return shellprocstatus === "running" ? "running" : "idle";
}

// ── ViewModel ────────────────────────────────────────────────────────────

/** Swarm's state behind its native pane tab (`swarmPaneTab`, swarm.tsx). */
export class SwarmViewModel {
    viewType = "swarm";
    blockId: string;




    private _subagents = createSignal<ActiveSubagent[]>([]);
    subagentsAtom: Accessor<ActiveSubagent[]> = this._subagents[0];
    private setSubagents: Setter<ActiveSubagent[]> = traced("subagents", this._subagents);

    // Active background shells (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20
    // Phase 1) — fetched separately via shell.ListActive; buildTree() groups
    // by block_id the same way it does for subagents/dispatches above.
    private _shells = createSignal<ActiveShell[]>([]);
    shellsAtom: Accessor<ActiveShell[]> = this._shells[0];
    private setShells: Setter<ActiveShell[]> = traced("shells", this._shells);

    // Cron jobs (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2) —
    // fetched separately via cron.ListActive; buildTree() groups by
    // block_id the same way it does for shells above.
    private _crons = createSignal<ActiveCron[]>([]);
    cronsAtom: Accessor<ActiveCron[]> = this._crons[0];
    private setCrons: Setter<ActiveCron[]> = traced("crons", this._crons);

    // Background tasks for the whole fleet, from the durable registry —
    // fetched via ListBackgroundTasksCommand (empty blockid); buildTree()
    // groups by block_id and nests each under the subagent that owns it.
    private _backgroundTasks = createSignal<BackgroundTaskView[]>([]);
    backgroundTasksAtom: Accessor<BackgroundTaskView[]> = this._backgroundTasks[0];
    private setBackgroundTasks: Setter<BackgroundTaskView[]> = traced("background", this._backgroundTasks);

    // Per-block todo checklist + in-flight tool, pushed by the backend's
    // `agent:progress` sweep. Push-only (no fetch-on-open counterpart): the
    // watcher republishes whenever the payload changes, so a Swarm opened
    // mid-session fills in within one sweep rather than needing its own RPC.
    private _progress = createSignal<Map<string, AgentProgress>>(new Map());
    progressAtom: Accessor<Map<string, AgentProgress>> = this._progress[0];
    private setProgress: Setter<Map<string, AgentProgress>> = traced("progress", this._progress);

    // AgentDispatches (SPEC §5) — one per Agent-tool-or-Workflow-tool call.
    // Fetched separately from subagentsAtom via subagent.ListDispatches;
    // buildTree() cross-references the two by dispatch_id/parent_block_id.
    private _dispatches = createSignal<AgentDispatch[]>([]);
    dispatchesAtom: Accessor<AgentDispatch[]> = this._dispatches[0];
    private setDispatches: Setter<AgentDispatch[]> = traced("dispatches", this._dispatches);

    // Map of blockId → "running" | "idle" — updated by controllerstatus events
    private _agentStatuses = createSignal<Map<string, "running" | "idle">>(new Map());
    agentStatusesAtom: Accessor<Map<string, "running" | "idle">> = this._agentStatuses[0];
    private setAgentStatuses: Setter<Map<string, "running" | "idle">> = traced("status", this._agentStatuses);

    // Ordered list of tracked block IDs (preserves server-side ordering)
    private _trackedBlockIds = createSignal<string[]>([]);
    trackedBlockIdsAtom: Accessor<string[]> = this._trackedBlockIds[0];
    private setTrackedBlockIds: Setter<string[]> = traced("tracked-ids", this._trackedBlockIds);

    // Fleet control (SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) — which
    // top-level agent blockIds are currently checked for a bulk action.
    // Ephemeral (resets on reload), same posture as _expandedAgentIds above.
    private _selectedBlockIds = createSignal<Set<string>>(new Set());
    selectedBlockIdsAtom: Accessor<Set<string>> = this._selectedBlockIds[0];
    private setSelectedBlockIds: Setter<Set<string>> = this._selectedBlockIds[1];

    // Result of the most recent bulk action, shown as a per-target success/failure list and never
    // collapsed to one aggregate toast (spec §3/§5.4: silent partial failure is the commonly cited
    // fleet-ops UX pitfall). Cleared only when the user dismisses the panel, so a failure list
    // can't vanish before it is read.
    private _lastFleetResult = createSignal<FleetResultEntry | null>(
        null,
    );
    lastFleetResultAtom: Accessor<FleetResultEntry | null> =
        this._lastFleetResult[0];
    private setLastFleetResult: Setter<FleetResultEntry | null> =
        this._lastFleetResult[1];

    private _fleetActionInFlight = createSignal<boolean>(false);
    fleetActionInFlightAtom: Accessor<boolean> = this._fleetActionInFlight[0];
    private setFleetActionInFlight: Setter<boolean> = this._fleetActionInFlight[1];

    private _loading = createSignal<boolean>(true);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading: Setter<boolean> = this._loading[1];

    // Rows the user has expanded, keyed by dispatchId (`WorkflowDispatchRow` and `SubagentRow` both
    // use toggleDispatchExpanded). Lives here, not in row-local state: `tree()` recomputes on every
    // trackedBlockIds / subagents / agentStatuses change (controllerstatus ticks are frequent
    // mid-turn) and rebuilds fresh wrapper objects each time, so `<For>`'s reference diffing
    // remounts rows far more often than the user changes anything, and local expand state would
    // collapse on the next unrelated tick. A stable string id here survives that churn.
    private _expandedIds = createSignal<Set<string>>(new Set());
    expandedIdsAtom: Accessor<Set<string>> = this._expandedIds[0];
    private setExpandedIds: Setter<Set<string>> = this._expandedIds[1];

    // Top-level agent rows share the same default (collapsed) and polarity (absent-means-collapsed)
    // as `_expandedIds`; this set tracks the agent blockIds explicitly expanded. Its own signal,
    // not folded into `_expandedIds`, because agent blockIds and subagent/workflow row keys are
    // different id spaces that could collide. Same residency reason as `_expandedIds`: `tree()`
    // rebuilds wrappers on every status tick.
    // Other instances' agents, for the sections below this instance's tree
    // (swarm-remote.ts). Null until the first answer; refreshed with the tracked
    // blocks' poll, since the source (the host-global registry) has no event.
    private _otherInstances = createSignal<SwarmOtherInstances | null>(null);
    otherInstancesAtom: Accessor<SwarmOtherInstances | null> = this._otherInstances[0];
    private _expandedAgentIds = createSignal<Set<string>>(new Set());
    expandedAgentIdsAtom: Accessor<Set<string>> = this._expandedAgentIds[0];
    private setExpandedAgentIds: Setter<Set<string>> = this._expandedAgentIds[1];

    // Rows the user has retired (dismissed): client-local, persisted to localStorage so a dismissal
    // survives reload (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3). Maps
    // rowKey -> the row's `lastEventAt` at the moment it was retired, not a bare set: buildTree()
    // suppresses a row only while its CURRENT `lastEventAt` matches the snapshot, so new activity
    // un-retires it with no explicit action
    // (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6).
    private _retiredRowKeys = createSignal<Map<string, number>>(loadRetiredRowKeysFromStorage());
    retiredRowKeysAtom: Accessor<Map<string, number>> = this._retiredRowKeys[0];
    private rawSetRetiredRowKeys: Setter<Map<string, number>> = traced("retired", this._retiredRowKeys);
    /** Every write to `_retiredRowKeys` must go through this — persists to
     *  localStorage alongside the in-memory update, so a raw
     *  `this.rawSetRetiredRowKeys` call is a bug (nothing enforces this at
     *  the type level, but every call site in this file goes through here). */
    private setRetiredRowKeys(updater: (prev: Map<string, number>) => Map<string, number>): void {
        this.rawSetRetiredRowKeys((prev) => {
            const next = updater(prev);
            if (next !== prev) saveRetiredRowKeysToStorage(next);
            return next;
        });
    }

    // Auto-linger countdown on a row's first clean-terminal appearance
    // (SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06), in the same rowKey space as
    // `_retiredRowKeys` (`subagentRowKey(agent_id)` or a Workflow dispatchId). `lastEventAt` is the
    // row's value at arm/last-reset time, the same snapshot comparison `_retiredRowKeys` uses, so
    // new activity un-arms instead of letting a stale countdown fire. `startedAt` (Date.now()) is
    // what the visible seconds render against. `pausedAt` is null while counting down;
    // `pauseCountdown()` sets it so the DISPLAYED number freezes (countdownSecondsRemaining
    // measures elapsed against `pausedAt` while it is non-null). Clearing only the timer stops the
    // auto-retire but leaves the wall-clock display counting down and clamping at 0 (#2440).
    // `resumeCountdown()` clears `pausedAt` and resets `startedAt` for a fresh 60s window.
    private _countdownState = createSignal<Map<string, CountdownEntry>>(new Map());
    countdownStateAtom: Accessor<Map<string, CountdownEntry>> = this._countdownState[0];
    private setCountdownState: Setter<Map<string, CountdownEntry>> = this._countdownState[1];
    private countdownTimers = new Map<string, ReturnType<typeof setTimeout>>();
    // The visible "Nn s" tick source is in the view (useTick(1000), frontend/app/hook/useTick.ts),
    // the ref-counted, auto-cleanup pattern this file already uses for relative-time displays, so
    // the ViewModel owns only the start / lastEventAt state, not a second timer.

    // One DispatchDetail (concatenated activity feed) per currently expanded row, Agent Tool (solo)
    // and Workflow alike (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4). Created lazily
    // on first expand: if the fetch+subscribe lifecycle lived in the row component, every
    // incidental remount (see expandedIds above) would refetch and resubscribe, potentially several
    // times a second while a row is open mid-turn.
    private dispatchDetailCache = new Map<string, DispatchDetail>();

    // Persisted across buildTree() calls so stabilizeGroupIdentity can reuse a WorkflowDispatch
    // wrapper whose content is unchanged. Without it, expandedIds / dispatchDetailCache don't help
    // a row nested in an expanded one: `<For>` remounts the whole WorkflowDispatchRow subtree
    // whenever the wrapper's reference changes.
    private groupIdentityCache = new Map<string, WorkflowDispatch>();

    private unsubs: (() => void)[] = [];
    // Per-block controllerstatus unsubs — cleaned up when block list refreshes
    private blockUnsubs: (() => void)[] = [];
    // The id set subscribeToBlockStatuses was last actually rebuilt for —
    // see loadTrackedBlocks's use of this (codex P2 on PR #3219).
    private lastSubscribedBlockIds: Set<string> = new Set();

    // The backend broadcasts one subagent:spawned / subagent:completed event per subagent file
    // (`process_jsonl_change`, and `flush_pending_dispatch_activity` for workflow members, in
    // `subagent_watcher/jsonl.rs`), and a backfill scan on pane reopen can fire dozens in a burst.
    // The loadSubagents() RPC is debounced here rather than batching the broadcasts, since
    // activity/subagent-source.ts (the ActivityDock) needs one event per subagent.
    private loadSubagentsDebounceTimer: ReturnType<typeof setTimeout> | undefined;
    private static readonly LOAD_SUBAGENTS_DEBOUNCE_MS = 150;

    // Same debounce shape as loadSubagentsDebounceTimer above, kept separate
    // since shell_node_create/shell_chunk bursts are unrelated to subagent/
    // dispatch events — no reason to couple their reload timing.
    private loadShellsDebounceTimer: ReturnType<typeof setTimeout> | undefined;
    private static readonly LOAD_SHELLS_DEBOUNCE_MS = 150;

    // Same debounce shape as loadShellsDebounceTimer above, for cron_changed
    // bursts (e.g. several jobs firing close together).
    private loadCronsDebounceTimer: ReturnType<typeof setTimeout> | undefined;
    private static readonly LOAD_CRONS_DEBOUNCE_MS = 150;

    // Same debounce shape again, for background-task-updated bursts (a
    // subagent can start or finish several background commands at once).
    private loadBackgroundTasksDebounceTimer: ReturnType<typeof setTimeout> | undefined;
    private static readonly LOAD_BACKGROUND_TASKS_DEBOUNCE_MS = 150;

    // Bounded self-healing safety net for trackedBlockIdsAtom — see the
    // comment at this timer's setInterval call site (in startTrackedBlocksPoll)
    // for why it exists alongside the five event-based refresh triggers.
    private trackedBlocksPollTimer: ReturnType<typeof setInterval> | undefined;
    private static readonly TRACKED_BLOCKS_POLL_MS = 12_000;

    constructor(blockId: string) {
        this.blockId = blockId;

        void this.loadAll();

        // Bounded backfill-naming backlog pass (#2829), fired once, here: this constructor runs
        // only when a human opens the Swarm pane (block-registry.ts's "swarm" view), never from the
        // headless per-agent-pane backfill scan that runs on every reopen. It resolves a capped
        // batch of whatever is unnamed in the historical backlog server-side; results stream back
        // through the subagent:named / dispatch:updated handlers below, so there is no response to
        // await.
        void callBackendService("subagent", "ResolveUnnamedBacklog", []);

        this.listenSubagentEvents();
        this.listenShellCronAndTaskEvents();
        this.listenProgressEvents();
        this.listenTrackedBlockEvents();
        this.startTrackedBlocksPoll();

        // term:osc_title / term:ambient_summary meta changes need no handler here: the block atom
        // in MOS updates reactively and the view's memo already reacts.
    }

    // Subagent and dispatch events: each one reloads the subagent list.
    private listenSubagentEvents(): void {
        const unsubSpawned = onMuxEvent(WpsEvent.SubagentSpawned, () => this.scheduleLoadSubagents());
        if (unsubSpawned) this.unsubs.push(unsubSpawned);

        const unsubCompleted = onMuxEvent(WpsEvent.SubagentCompleted, () => this.scheduleLoadSubagents());
        if (unsubCompleted) this.unsubs.push(unsubCompleted);

        // A known subagent's `tool_use_id` arrived after its spawn (the CLI
        // writes the sidecar separately) — reload so its background tasks
        // move from the agent-level bucket to under its row.
        const unsubUpdated = onMuxEvent(WpsEvent.SubagentUpdated, () => this.scheduleLoadSubagents());
        if (unsubUpdated) this.unsubs.push(unsubUpdated);

        // One or more subagents just reconciled active -> abandoned, live
        // (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 Phase A, #2234: the instant
        // their parent's turn ends) or at reopen. Reload so the row's display status
        // (subagentDisplayStatus) updates without waiting for an unrelated event.
        const unsubAbandoned = onMuxEvent(WpsEvent.SubagentAbandoned, () => this.scheduleLoadSubagents());
        if (unsubAbandoned) this.unsubs.push(unsubAbandoned);

        // dispatch:updated fires for Solo and Workflow kinds (SPEC §5); a member count or status
        // change on either warrants the same reload as a subagent spawn or completion (a workflow
        // member spawning IS a dispatch update).
        const unsubDispatchUpdated = onMuxEvent(WpsEvent.DispatchUpdated, () => this.scheduleLoadSubagents());
        if (unsubDispatchUpdated) this.unsubs.push(unsubDispatchUpdated);

        // A closed block's subagents/dispatches were just pruned server-side
        // (SubagentWatcher::prune_block, backstopped against BlockDeleted/
        // TabDeleted/WorkspaceDeleted) — reload so the ghost row this used
        // to leave behind (visible until srv restart) disappears promptly.
        const unsubBlockPruned = onMuxEvent(WpsEvent.SubagentBlockPruned, () => this.scheduleLoadSubagents());
        if (unsubBlockPruned) this.unsubs.push(unsubBlockPruned);
    }

    // Shell, cron and background-task events: each reloads its own list.
    private listenShellCronAndTaskEvents(): void {
        // Shell bucket (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 1).
        // shell_node_create — a new shell appeared, reload the active list.
        const unsubShellCreate = onMuxEvent(WpsEvent.ShellNodeCreate, () => this.scheduleLoadShells());
        if (unsubShellCreate) this.unsubs.push(unsubShellCreate);

        // shell_chunk fires per output line too — only reload on the
        // terminal "exit" op, not every line of stdout/stderr (that would
        // re-fetch the whole active-shells list on every chunk).
        const unsubShellChunk = onMuxEvent(WpsEvent.ShellChunk, (data) => {
            if (data.op === "exit") this.scheduleLoadShells();
        });
        if (unsubShellChunk) this.unsubs.push(unsubShellChunk);

        // Cron bucket (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2).
        // Payload-free — any create/fire/pause/resume/delete just triggers a
        // full reload of the (already-cheap, unfiltered) active list.
        const unsubCronChanged = onMuxEvent(WpsEvent.CronChanged, () => this.scheduleLoadCrons());
        if (unsubCronChanged) this.unsubs.push(unsubCronChanged);

        // Background tasks. Payload-free invalidation, scoped to the block
        // whose registry changed; the Swarm listens to every block (no scope)
        // and reloads the fleet list.
        const unsubBackgroundTask = onMuxEvent(WpsEvent.BackgroundTaskUpdated, () => this.scheduleLoadBackgroundTasks());
        if (unsubBackgroundTask) this.unsubs.push(unsubBackgroundTask);
    }

    // Events that carry their whole payload: progress, and a subagent's name.
    private listenProgressEvents(): void {
        // Todo checklist + in-flight tool. Unlike the buckets above this
        // carries its whole payload on the event, so there's nothing to
        // reload — store it keyed by block and let buildTree read it.
        const unsubProgress = onMuxEvent(WpsEvent.AgentProgress, (data) => {
            const blockId = data.blockId;
            if (!blockId) return;
            const next: AgentProgress = {
                todos: data.todos.filter((t) => t.text.length > 0),
                todosTruncated: data.todosTruncated,
                todosPartial: data.todosPartial,
                currentTool: data.currentTool,
            };
            // A new Map (not a mutation) so Solid sees a new reference and buildTree re-runs, but only for a
            // change: the same progress again leaves the Map alone (swarm-updates.ts).
            this.setProgress((prev) => withAgentProgress(prev, blockId, next));
        });
        if (unsubProgress) this.unsubs.push(unsubProgress);

        // Patch display_name in place (not a full loadSubagents() reload) so
        // every client watching this session picks up a generated name —
        // not just the one whose expand click triggered subagent.GenerateName.
        const unsubNamed = onSubagentNamed((agentId, displayName) =>
            this.setSubagents((prev) => withSubagentName(prev, agentId, displayName))
        );
        if (unsubNamed) this.unsubs.push(unsubNamed);
    }

    // Events that change which blocks are tracked: each re-reads the list.
    private listenTrackedBlockEvents(): void {
        // When process trackers change, refresh the block list
        const unsubProcAdded = onMuxEvent(WpsEvent.AgentProcessAdded, () => void this.loadTrackedBlocks());
        if (unsubProcAdded) this.unsubs.push(unsubProcAdded);

        const unsubProcExited = onMuxEvent(WpsEvent.AgentProcessExited, () => void this.loadTrackedBlocks());
        if (unsubProcExited) this.unsubs.push(unsubProcExited);

        // When a reactive-handler agent (Claude Code pane) registers or
        // unregisters, refresh the block list. These events are distinct from
        // agent:process-added / agent:process-exited so useTrackedProcesses
        // doesn't treat reactive registrations as phantom OS processes.
        const unsubReactiveReg = onMuxEvent(WpsEvent.AgentReactiveRegistered, () => void this.loadTrackedBlocks());
        if (unsubReactiveReg) this.unsubs.push(unsubReactiveReg);

        const unsubReactiveUnreg = onMuxEvent(WpsEvent.AgentReactiveUnregistered, () => void this.loadTrackedBlocks());
        if (unsubReactiveUnreg) this.unsubs.push(unsubReactiveUnreg);

        // Authoritative signal, in addition to the four above: published directly from
        // blockcontroller::register_controller / delete_controller, the functions that determine
        // what agent.tracked-blocks returns. The four above are process_tracker's 2s poll-and-diff
        // and the separate `reactive` registration map, neither guaranteed to fire in step with a
        // controller (de)registering, which is how a restored agent's controller could register
        // server-side while this list stayed stale client-side indefinitely
        // (docs/reports/REPORT_SWARM_MOUNT_DEPENDENT_TRACKING_GAP_2026_09_15.md).
        const unsubTrackedBlocksChanged = onMuxEvent(WpsEvent.ProcessBrokerTrackedBlocksChanged, () => void this.loadTrackedBlocks());
        if (unsubTrackedBlocksChanged) this.unsubs.push(unsubTrackedBlocksChanged);
    }

    // The safety-net poll behind those events.
    private startTrackedBlocksPoll(): void {
        // Bounded self-healing safety net: re-poll whether or not any of the five event triggers
        // above fired, so a gap in event wiring not yet discovered can't leave this list stale
        // forever. Cheap: the RPC is a HashMap clone + Vec filter server-side, not a per-block
        // probe.
        this.trackedBlocksPollTimer = setInterval(() => {
            void this.loadTrackedBlocks();
            void this.loadOtherInstances();
        }, SwarmViewModel.TRACKED_BLOCKS_POLL_MS);
    }

    loadAll = async (): Promise<void> => {
        this.setLoading(true);
        try {
            const [, subagentsOk, dispatchesOk] = await Promise.all([
                this.loadTrackedBlocks(),
                this.loadSubagents(),
                this.loadDispatches(),
                this.loadShells(),
                this.loadCrons(),
                this.loadBackgroundTasks(),
                this.loadOtherInstances(),
            ]);
            // A transient ListActive/ListDispatches failure is swallowed by its loader (silent
            // catch below) and leaves the atom stale or empty; pruning against that would treat
            // "not loaded yet" as "gone" and permanently delete persisted retire entries (#2677).
            // Prune only once BOTH sources reflect real backend state.
            if (subagentsOk && dispatchesOk) {
                this.pruneRetiredRowKeys();
            }
            this.reconcileCountdowns();
        } finally {
            this.setLoading(false);
        }
    };

    loadTrackedBlocks = async (): Promise<void> => {
        try {
            const { block_ids } = await RpcApi.AgentTrackedBlocksCommand(TabRpcClient, {});
            const ids = refetchAbsentWindowBlocks(block_ids ?? []); // also reads again a block this window holds as missing
            if (!sameList(untrack(this.trackedBlockIdsAtom), ids)) this.setTrackedBlockIds(ids);
            // subscribeToBlockStatuses rebuilds every per-block status subscription (each unsub/sub
            // flushes its own MPS eventsub command) plus a fresh GetControllerStatus per block: needed
            // when membership changed, pure overhead when it didn't (#3219). The safety-net poll below
            // calls this every TRACKED_BLOCKS_POLL_MS regardless, so skip the rebuild unless the id
            // SET (not order — server-side ordering can shift without membership changing) differs.
            const changed =
                ids.length !== this.lastSubscribedBlockIds.size || ids.some((id) => !this.lastSubscribedBlockIds.has(id));
            if (changed) {
                this.lastSubscribedBlockIds = new Set(ids);
                this.subscribeToBlockStatuses(ids);
            }
        } catch {
            // silent — safe default is empty tree
        }
    };

    /** @returns whether the RPC actually succeeded — callers use this to
     *  decide whether it's safe to prune retired-row keys against the
     *  resulting atom (codex P2 on PR #2677 — see `loadAll`/
     *  `scheduleLoadSubagents`). */
    loadSubagents = async (): Promise<boolean> => {
        try {
            const result = await callBackendService("subagent", "ListActive", []);
            const list = (result as ActiveSubagent[]) ?? [];
            this.setSubagents((prev) => mergeSubagentsPreservingIdentity(prev, list));
            return true;
        } catch {
            // silently ignore
            return false;
        }
    };

    /** @returns whether the RPC actually succeeded — see `loadSubagents`'s
     *  doc comment. */
    loadDispatches = async (): Promise<boolean> => {
        try {
            const result = await callBackendService("subagent", "ListDispatches", []);
            this.setDispatches((result as AgentDispatch[]) ?? []);
            return true;
        } catch {
            // silently ignore
            return false;
        }
    };

    loadShells = async (): Promise<void> => {
        try {
            const result = await callBackendService("shell", "ListActive", []);
            this.setShells((result as ActiveShell[]) ?? []);
        } catch {
            // silently ignore
        }
    };

    loadCrons = async (): Promise<void> => {
        try {
            const result = await callBackendService("cron", "ListActive", []);
            this.setCrons((result as ActiveCron[]) ?? []);
        } catch {
            // silently ignore
        }
    };

    loadBackgroundTasks = async (): Promise<void> => {
        try {
            const result = await RpcApi.ListBackgroundTasksCommand(TabRpcClient, { blockid: "" });
            this.setBackgroundTasks(result ?? []);
        } catch {
            // silently ignore — same posture as the other loaders
        }
    };

    scheduleLoadBackgroundTasks = (): void => {
        if (this.loadBackgroundTasksDebounceTimer !== undefined) {
            clearTimeout(this.loadBackgroundTasksDebounceTimer);
        }
        this.loadBackgroundTasksDebounceTimer = setTimeout(() => {
            this.loadBackgroundTasksDebounceTimer = undefined;
            void this.loadBackgroundTasks();
        }, SwarmViewModel.LOAD_BACKGROUND_TASKS_DEBOUNCE_MS);
    };

    // Coalesces a burst of subagent:spawned / subagent:completed / dispatch:updated events (a
    // backfill scan on reopen, a large workflow dispatch spawning many members) into one reload
    // after the burst settles, not one RPC pair per event.
    scheduleLoadSubagents = (): void => {
        if (this.loadSubagentsDebounceTimer !== undefined) {
            clearTimeout(this.loadSubagentsDebounceTimer);
        }
        this.loadSubagentsDebounceTimer = setTimeout(() => {
            this.loadSubagentsDebounceTimer = undefined;
            void Promise.all([this.loadSubagents(), this.loadDispatches()]).then(([subagentsOk, dispatchesOk]) => {
                // See loadAll's identical guard — don't prune against a
                // transiently-failed fetch.
                if (subagentsOk && dispatchesOk) {
                    this.pruneRetiredRowKeys();
                }
                this.reconcileCountdowns();
            });

            // (#2829, #2830) The constructor's one-shot ResolveUnnamedBacklog call covers only what
            // is backfilled when the Swarm pane opens. A backfill landing on an already-open pane
            // (another agent pane reopening) reaches this debounced handler via subagent:spawned /
            // dispatch:updated but never re-fires the resolver, so those rows stay raw-slugged
            // until the view model is rebuilt. Re-firing here closes that gap and costs little when
            // nothing is new: the backend's naming_triggered claim makes a drained backlog a fast
            // no-op scan, not a model call.
            void callBackendService("subagent", "ResolveUnnamedBacklog", []);
        }, SwarmViewModel.LOAD_SUBAGENTS_DEBOUNCE_MS);
    };

    // Coalesces a burst of shell_node_create/shell_chunk(exit) events (e.g.
    // several shells starting/exiting close together) into a single reload.
    scheduleLoadShells = (): void => {
        if (this.loadShellsDebounceTimer !== undefined) {
            clearTimeout(this.loadShellsDebounceTimer);
        }
        this.loadShellsDebounceTimer = setTimeout(() => {
            this.loadShellsDebounceTimer = undefined;
            void this.loadShells();
        }, SwarmViewModel.LOAD_SHELLS_DEBOUNCE_MS);
    };

    // Coalesces a burst of cron_changed events into a single reload.
    scheduleLoadCrons = (): void => {
        if (this.loadCronsDebounceTimer !== undefined) {
            clearTimeout(this.loadCronsDebounceTimer);
        }
        this.loadCronsDebounceTimer = setTimeout(() => {
            this.loadCronsDebounceTimer = undefined;
            void this.loadCrons();
        }, SwarmViewModel.LOAD_CRONS_DEBOUNCE_MS);
    };

    /**
     * Drop `_retiredRowKeys` entries for rows no longer in live state (see `pruneRetiredEntries`).
     * Called after `loadSubagents` / `loadDispatches` settle, as a plain state update and not from
     * inside `buildTree()`'s read of `retiredRowKeysAtom`, which would be a reactive write during a
     * read.
     */
    private pruneRetiredRowKeys(): void {
        const liveKeys = new Set<string>();
        const subagents = this.subagentsAtom();
        const dispatches = this.dispatchesAtom();
        for (const s of subagents) liveKeys.add(subagentRowKey(s.agent_id));
        const knownWorkflowDispatchIds = new Set<string>();
        for (const d of dispatches) {
            if (d.kind === "workflow") {
                liveKeys.add(d.dispatch_id);
                knownWorkflowDispatchIds.add(d.dispatch_id);
            }
        }
        // A placeholder WorkflowDispatch row (`buildDispatchBuckets`'s `orphanedWorkflowMembers`
        // fallback, rendered when `dispatchesAtom` lags this dispatch_id) is retired under its own
        // `dispatch_id` like a real one. If an orphaned member's dispatch_id weren't also treated
        // as live here, it would never be in `liveKeys`, and the next refresh's pruning would
        // delete the dismissal and resurrect the row (#2677). Mirrors buildDispatchBuckets's orphan
        // detection.
        for (const s of subagents) {
            if (!s.dispatch_id.startsWith("solo:") && !knownWorkflowDispatchIds.has(s.dispatch_id)) {
                liveKeys.add(s.dispatch_id);
            }
        }
        this.setRetiredRowKeys((prev) => pruneRetiredEntries(prev, liveKeys));
    }

    // ── Row expand/collapse (workflow groups + subagent detail) ──────────

    isExpanded(id: string): boolean {
        return this.expandedIdsAtom().has(id);
    }

    /** Generic toggle — used by WorkflowGroupRow (no data-fetching side effects). */
    toggleExpanded(id: string): void {
        this.setExpandedIds((prev) => {
            const next = new Set(prev);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
    }

    /** Top-level AgentRow collapse — same default-collapsed,
     *  absent-means-collapsed semantics as isExpanded/toggleExpanded above,
     *  backed by its own _expandedAgentIds set (see there for why). */
    isAgentCollapsed(blockId: string): boolean {
        return !this.expandedAgentIdsAtom().has(blockId);
    }

    /** Fetch the other instances' agents. A failure keeps the last answer: these
     *  sections are informational and must never disturb this instance's tree. */
    loadOtherInstances = async (): Promise<void> => {
        try {
            const result = await RpcApi.SwarmOtherInstancesCommand(TabRpcClient, {});
            this._otherInstances[1](result ?? null);
        } catch {
            // Older srv without the command, or a transient failure.
        }
    };

    /** Keeps in the selection only what the Swarm lists: `localKeys` (this
     *  instance's selectable agents, from the view's tree) and the remote agents
     *  not gone stale. An agent whose pane closed, or a remote one that left,
     *  would otherwise stay selected with no checkbox showing it, and count in
     *  the toolbar (docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §2). The view
     *  calls this whenever its list changes. */
    pruneSelection(localKeys: Iterable<string>): void {
        const listed = new Set(localKeys);
        for (const [key, target] of remoteFleetTargets(remoteSections(this.otherInstancesAtom()))) {
            if (!target.stale) listed.add(key);
        }
        const selected = this.selectedBlockIdsAtom();
        if ([...selected].every((key) => listed.has(key))) return;
        this.setSelectedBlockIds(new Set([...selected].filter((key) => listed.has(key))));
    }

    toggleAgentCollapsed(blockId: string): void {
        this.setExpandedAgentIds((prev) => {
            const next = new Set(prev);
            if (next.has(blockId)) next.delete(blockId);
            else next.add(blockId);
            return next;
        });
    }

    // ── Fleet control (SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) ──────

    isSelected(blockId: string): boolean {
        return this.selectedBlockIdsAtom().has(blockId);
    }

    toggleSelected(blockId: string): void {
        this.setSelectedBlockIds((prev) => {
            const next = new Set(prev);
            if (next.has(blockId)) next.delete(blockId);
            else next.add(blockId);
            return next;
        });
    }

    selectAll(blockIds: string[]): void {
        this.setSelectedBlockIds(new Set(blockIds));
    }

    /** Adds `keys` to the selection, or removes them: a machine's header
     *  checkbox selects exactly that machine's agents. */
    setManySelected(keys: string[], on: boolean): void {
        this.setSelectedBlockIds((prev) => {
            const next = new Set(prev);
            for (const key of keys) {
                if (on) next.add(key);
                else next.delete(key);
            }
            return next;
        });
    }

    /** Everything that can be selected: this instance's agents and the other
     *  instances' (docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §4). */
    fleetTargets(): Map<string, FleetTargetInfo> {
        return new Map([
            ...localFleetTargets(this.buildTree()),
            ...remoteFleetTargets(remoteSections(this.otherInstancesAtom())),
        ]);
    }

    clearSelection(): void {
        this.setSelectedBlockIds(new Set<string>());
    }

    /** Non-destructive: executes immediately, no confirmation — see spec
     *  §5.2/§3 (reserve blocking confirms for irreversible actions). */
    broadcastToSelection(message: string): Promise<FleetActionResult> {
        return this.runFleetAction("broadcast", "broadcast", (targets) => RpcApi.FleetBroadcastCommand(TabRpcClient, { targets, message }));
    }

    /** Destructive — callers must confirm with the user first, showing the
     *  exact resolved target count (spec §3: never hide scope, never "stop
     *  selected" without the concrete list/count). Clears the selection on
     *  completion regardless of outcome, since the stopped panes are no
     *  longer a meaningful target set to keep checked. */
    async bulkStopSelection(opts?: { signal?: string; staged?: FleetStagePlan }): Promise<FleetActionResult> {
        try {
            return await this.runFleetAction("bulk-stop", "stop", (targets) => RpcApi.FleetBulkStopCommand(TabRpcClient, { targets, ...opts }, { timeout: 600_000 }));
        } finally {
            this.clearSelection();
        }
    }

    /** Sets the Supervisor's mode on the selection. Not destructive, so no
     *  confirmation, like broadcast; every target's outcome is reported (an
     *  agent that isn't Claude, or isn't on this AgentMux, says why). */
    superviseSelection(mode: SupervisorMode): Promise<FleetActionResult> {
        return this.runFleetAction("supervisor", "supervise", (targets) => RpcApi.SupervisorSetModeCommand(TabRpcClient, { targets, mode }));
    }

    /** One fleet action on the selection: the targets it can reach go to
     *  `call`, the rest are reported as failed with the reason, and the result
     *  panel shows every target by name. */
    private async runFleetAction(
        action: FleetResultEntry["action"],
        reach: FleetAction,
        call: (targets: string[]) => Promise<FleetActionResult>
    ): Promise<FleetActionResult> {
        const selected = actionableKeys(this, this.selectedBlockIdsAtom());
        const known = this.fleetTargets();
        const { reachable: targets, unreachable } = partitionTargets(selected, reach, known);
        this.setFleetActionInFlight(true);
        try {
            const result = withUnreachable(targets.length > 0 ? await call(targets) : EMPTY_FLEET_RESULT, unreachable);
            this.setLastFleetResult({ action, result, labels: labelsOf(selected, known) });
            return result;
        } finally {
            this.setFleetActionInFlight(false);
        }
    }

    dismissFleetResult(): void {
        this.setLastFleetResult(null);
    }

    /**
     * Retire (dismiss) a row: `rowKey` is `subagentRowKey(agent_id)` for an Agent Tool row or the
     * raw `dispatchId` for a `WorkflowDispatchRow` (the keys `buildTree()`'s filter checks).
     * `lastEventAt` is the row's value at retire time (see `_retiredRowKeys`). Expose it only for a
     * terminal-status row (`"idle"` / `"interrupted"`). Also clears any pending auto-linger
     * countdown for the row, so no timer fires later against an already-retired key.
     */
    retireRow(rowKey: string, lastEventAt: number): void {
        this.setRetiredRowKeys((prev) => withRetiredEntry(prev, rowKey, lastEventAt));
        this.clearCountdown(rowKey);
    }

    /**
     * Bulk "Clear completed": retires every visible terminal-status row across every block in one
     * action, instead of one Retire click per row on a large historical backlog (useful since
     * `_retiredRowKeys` survives reload). One batched `setRetiredRowKeys` write and one
     * countdown-clear pass for the whole set, not N `retireRow` calls: N calls meant N
     * JSON.stringify + localStorage.setItem round trips for one action (#2677;
     * SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3).
     */
    retireAllCompleted(): void {
        const rows = collectClearableRows(this.buildTree());
        if (rows.length === 0) return;
        this.setRetiredRowKeys((prev) => {
            const next = new Map(prev);
            for (const { rowKey, lastEventAt } of rows) next.set(rowKey, lastEventAt);
            return next;
        });
        for (const { rowKey } of rows) this.clearCountdown(rowKey);
    }

    // ── Auto-linger countdown (SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06) ──

    private armCountdownTimer(rowKey: string): void {
        const existingTimer = this.countdownTimers.get(rowKey);
        if (existingTimer !== undefined) clearTimeout(existingTimer);
        const timer = setTimeout(() => {
            this.countdownTimers.delete(rowKey);
            const entry = this.countdownStateAtom().get(rowKey);
            if (entry === undefined) return; // already cleared — manually retired, or un-armed by new activity
            this.retireRow(rowKey, entry.lastEventAt);
        }, AUTO_RETIRE_DELAY_MS);
        this.countdownTimers.set(rowKey, timer);
    }

    /** Arm a fresh 60s countdown for `rowKey`, or leave an already-running
     *  one alone if it's counting down against the SAME `lastEventAt` (so
     *  reconcileCountdowns(), called on every subagent/dispatch reload,
     *  doesn't restart the visible number on every unrelated refresh). */
    private armCountdown(rowKey: string, lastEventAt: number): void {
        const existing = this.countdownStateAtom().get(rowKey);
        if (existing && existing.lastEventAt === lastEventAt) return;
        this.setCountdownState((prev) => {
            const next = new Map(prev);
            next.set(rowKey, { lastEventAt, startedAt: Date.now(), pausedAt: null });
            return next;
        });
        this.armCountdownTimer(rowKey);
    }

    /** Clear a row's countdown entirely — pending timer + visible state.
     *  Called on manual retire (short-circuit), new activity arriving on
     *  the same key (un-arm — see reconcileCountdowns), or the timer's own
     *  fire (via retireRow, which calls back into this). */
    private clearCountdown(rowKey: string): void {
        const timer = this.countdownTimers.get(rowKey);
        if (timer !== undefined) {
            clearTimeout(timer);
            this.countdownTimers.delete(rowKey);
        }
        if (!this.countdownStateAtom().has(rowKey)) return;
        this.setCountdownState((prev) => {
            const next = new Map(prev);
            next.delete(rowKey);
            return next;
        });
    }

    /** Pause `rowKey`'s countdown on hover — clears the pending timer (so
     *  the row can't auto-retire while being read) AND stamps `pausedAt`
     *  so the DISPLAYED number actually freezes there too, rather than
     *  continuing to count down against the live clock while merely the
     *  timer is stopped (reagentx P1 on #2440 — see the entry's own doc
     *  comment). No-op if this row isn't actually counting down. */
    pauseCountdown(rowKey: string): void {
        const timer = this.countdownTimers.get(rowKey);
        if (timer === undefined) return;
        clearTimeout(timer);
        this.countdownTimers.delete(rowKey);
        this.setCountdownState((prev) => {
            const entry = prev.get(rowKey);
            if (entry === undefined) return prev;
            const next = new Map(prev);
            next.set(rowKey, { ...entry, pausedAt: Date.now() });
            return next;
        });
    }

    /** Resume `rowKey`'s countdown on mouse-leave with a FRESH 60s window,
     *  not the remaining time from before the pause — see
     *  SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md's "resume ≠
     *  resume mid-count" decision. Clears `pausedAt` so the display resumes
     *  computing against the live clock again. No-op if the row un-retired
     *  itself (or was manually dismissed) while paused. */
    resumeCountdown(rowKey: string): void {
        const entry = this.countdownStateAtom().get(rowKey);
        if (entry === undefined) return;
        this.setCountdownState((prev) => {
            const next = new Map(prev);
            next.set(rowKey, { ...entry, startedAt: Date.now(), pausedAt: null });
            return next;
        });
        this.armCountdownTimer(rowKey);
    }

    /**
     * Arm/clear countdowns against current subagent/dispatch state, called after
     * loadSubagents()/loadDispatches() settle (same timing as pruneRetiredRowKeys()). In scope:
     * agentToolRows/workflowRows only (the spec's "out of scope: shellRows/cronRows").
     *
     * "agentToolRows" means `buildDispatchBuckets()`'s actual `agentToolRows` output (solo +
     * orphaned-workflow-member subagents), NOT the raw `subagentsAtom()` list, which also holds
     * every NORMAL member of a still-tracked Workflow dispatch. Those never get their own
     * `SubagentRow` (SPEC §7: a dispatch that large can't hold thousands of member rows, hence
     * `WorkflowDispatchRow`'s aggregate-only display). Arming a countdown per completed member of,
     * say, a 1,030-member workflow would bring back the per-member `setTimeout`/state-entry cost
     * this design avoids, AND permanently hide a member that later surfaces through the
     * `orphanedWorkflowMembers` fallback (a stale or failed `ListDispatches`): `retireRow` would
     * already have retired its `subagentRowKey` against its unchanged `last_event_at`, so
     * `filterRetired` would suppress it forever just when it needs the fallback to stay visible
     * (#2440). Reusing `buildDispatchBuckets` rather than re-deriving its solo/orphaned logic keeps
     * the arming scope in sync with what renders.
     *
     * A subagent's `status === "completed"` is exactly `subagentDisplayStatus()`'s (swarm-view.tsx)
     * `"idle"` case regardless of parent status (`"active"` / `"abandoned"` never map to `"idle"`),
     * so checking it directly arms the same set without duplicating that function's parent-status
     * branching or importing across the model/view boundary. A `"completed"` Workflow dispatch is
     * the bucket's equivalent terminal state. Neither bucket has a distinct dispatch-level "failed"
     * status to exclude, and an `"abandoned"` (interrupted) subagent is never `"completed"`, so it
     * is already outside `liveTerminal`.
     */
    private reconcileCountdowns(): void {
        const dispatches = this.dispatchesAtom();
        const { agentToolRows } = buildDispatchBuckets(dispatches, this.subagentsAtom());

        const liveTerminal = new Map<string, number>();
        for (const sub of agentToolRows) {
            if (sub.status === "completed") liveTerminal.set(subagentRowKey(sub.agent_id), sub.last_event_at);
        }
        for (const d of dispatches) {
            if (d.kind === "workflow" && d.status === "completed") liveTerminal.set(d.dispatch_id, d.last_event_at);
        }

        for (const [rowKey, entry] of this.countdownStateAtom()) {
            const currentLastEventAt = liveTerminal.get(rowKey);
            if (currentLastEventAt === undefined || currentLastEventAt !== entry.lastEventAt) {
                this.clearCountdown(rowKey);
            }
        }
        for (const [rowKey, lastEventAt] of countdownCandidates(liveTerminal, this.retiredRowKeysAtom())) {
            this.armCountdown(rowKey, lastEventAt);
        }
    }

    /**
     * Every row's toggle. `rowKey` must be a per-ROW-unique identity, not necessarily the row's
     * `dispatch_id`: a `WorkflowDispatchRow`'s `dispatch_id` is 1:1 with its row, but an Agent Tool
     * row's is not, because an orphaned workflow member (falling back into the Agent Tool bucket
     * while `ListDispatches` lags) shares its real `dispatch_id` with every sibling still waiting
     * on the same lag. A single-subagent row must pass a per-row-unique key (e.g.
     * `agent:${sub.agent_id}`; see `getDispatchDetail`). Tears down the cached DispatchDetail (and
     * its WS subscriptions) on collapse, so a row opened once doesn't keep a live subscription
     * forever (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4).
     */
    toggleDispatchExpanded(rowKey: string): void {
        const wasExpanded = this.isExpanded(rowKey);
        this.toggleExpanded(rowKey);
        if (wasExpanded) {
            this.dispatchDetailCache.get(rowKey)?.dispose();
            this.dispatchDetailCache.delete(rowKey);
        }
    }

    /**
     * `rowKey`: cache identity for THIS row, unique per row (see `toggleDispatchExpanded` for why
     * it can't be `dispatchId` for an Agent Tool row). Two sibling rows sharing a `dispatchId` but
     * cached under distinct `rowKey`s used to subscribe to the same live `dispatch:activity`
     * broadcast unfiltered, so each rendered every other sibling's events too.
     *
     * `dispatchId`: the real dispatch_id to subscribe to for live `dispatch:activity` events; it
     * may be shared across sibling rows, since `createDispatchDetail` filters live events to
     * `backfillAgentId` when one is given.
     *
     * `backfillAgentId`: pass the row's own single `ActiveSubagent.agent_id` whenever the caller
     * renders exactly one subagent's row (an Agent Tool row, including an orphaned workflow
     * member), not just when `dispatchId` starts with `"solo:"`, which dropped an orphaned member's
     * `GetHistory` backfill (#2232). Omit it for a genuine `WorkflowDispatchRow`, which represents
     * many members and has no one agent to backfill or filter to.
     */
    getDispatchDetail(rowKey: string, dispatchId: string, backfillAgentId?: string): DispatchDetail {
        let detail = this.dispatchDetailCache.get(rowKey);
        if (!detail) {
            detail = createDispatchDetail(dispatchId, backfillAgentId);
            this.dispatchDetailCache.set(rowKey, detail);
        }
        return detail;
    }

    // Subscribe to controllerstatus events for each tracked block and
    // seed the initial status from GetControllerStatus (not assumed "idle").
    // Tears down old per-block subs first so we don't leak on block-list refresh.
    private subscribeToBlockStatuses(blockIds: string[]): void {
        for (const unsub of this.blockUnsubs) unsub();
        this.blockUnsubs = [];

        // Preserve prior status for existing blocks; only default new blocks to
        // "idle". This prevents a running→idle→running flicker when process
        // events cause loadTrackedBlocks to re-run while an agent is working.
        this.setAgentStatuses((prev) =>
            new Map(blockIds.map((id) => [id, prev.get(id) ?? ("idle" as const)]))
        );

        // Prune pushed progress for blocks that are gone, mirroring the backend watcher's per-sweep
        // `retain` (#2952). `agent:progress` is push-only, so the Map would keep an entry for every
        // block that ever reported, unbounded over a long session of ephemeral panes. The
        // block-list refresh is the right place rather than the `block_pruned` event: it is the
        // general case, and the statuses Map above is already rebuilt from the same authoritative
        // list.
        const live = new Set(blockIds);
        this.setProgress((prev) => {
            if (![...prev.keys()].some((id) => !live.has(id))) return prev;
            return new Map([...prev].filter(([id]) => live.has(id)));
        });

        for (const blockId of blockIds) {
            // Fetch current status — don't assume idle for already-running agents.
            void BlockService.GetControllerStatus(blockId)
                .then((rts) => {
                    const status = derivedRunningStatus(rts?.is_agent_pane, rts?.turn_active, rts?.shellprocstatus);
                    this.setAgentStatuses((prev) => withAgentStatus(prev, blockId, status));
                })
                .catch(() => {/* keep idle default */});

            const scope = MOS.makeORef("block", blockId);
            const unsub = onMuxEvent(WpsEvent.ControllerStatus, (data) => {
                    const next = derivedRunningStatus(data.is_agent_pane, data.turn_active, data.shellprocstatus);
                    this.setAgentStatuses((prev) => withAgentStatus(prev, blockId, next));
                }, scope);
            if (unsub) this.blockUnsubs.push(unsub);
        }
    }

    // Build the derived tree from flat atoms — called by the view via createMemo
    buildTree(): AgentTreeNode[] {
        const blockIds = this.trackedBlockIdsAtom();
        const subagents = this.subagentsAtom();
        const dispatches = this.dispatchesAtom();
        const shells = this.shellsAtom();
        const crons = this.cronsAtom();
        const backgroundTasks = this.backgroundTasksAtom();
        const statuses = this.agentStatusesAtom();
        const progressByBlock = this.progressAtom();

        // Include parent block IDs from subagents as fallback for agent panes
        // that registered subagents before their own registration propagated.
        const parentIds = subagents.map((s) => s.parent_block_id).filter(Boolean);
        const allBlockIds = [...new Set([...blockIds, ...parentIds])];

        // Collected from the WorkflowDispatch rows actually produced below, not derived from the
        // raw dispatch list, so stabilizeGroupIdentity's cache retains keys only for dispatches
        // present this pass (harmless either way, but this is the precise set).
        const liveGroupKeys = new Set<string>();
        const retired = this.retiredRowKeysAtom();
        const nodes = allBlockIds.flatMap((blockId) => {
            const blockAtom = MOS.getMuxObjectAtom<Block>(`block:${blockId}`);
            const block = blockAtom();
            // isLoading distinguishes "this oref hasn't resolved yet" from "resolved to nothing":
            // both read as block == null, but only the latter means the id is genuinely phantom.
            // getMuxObjectLoadingAtom returns `null` while loading and `false` once GetObject has
            // resolved either way, found or a definitive "not found" (mos.ts's `startFetch`; any
            // other fetch error leaves it `null`).
            const isLoading = MOS.getMuxObjectLoadingAtom(`block:${blockId}`)() !== false;
            if (!hasRenderableBlock(block, isLoading)) return [];
            const agentName =
                (block?.meta?.["agentName"] as string | undefined)?.trim() ||
                "Agent";
            const agentProvider =
                (block?.meta?.["agentProvider"] as string | undefined)?.trim() || null;
            // The same function the pane tab tooltip uses, so the two cannot disagree.
            const activitySummary = readSwarmSummary(block?.meta);
            const contextTokens = swarmContextTokens(block?.meta);
            const agentStatus = statuses.get(blockId) ?? "idle";
            const { agentToolRows: rawAgentToolRows, workflowRows: rawWorkflowRows } = buildDispatchBuckets(
                dispatches.filter((d) => d.parent_block_id === blockId),
                subagents.filter((s) => s.parent_block_id === blockId)
            );
            for (const w of rawWorkflowRows) liveGroupKeys.add(groupCacheKey(w));
            const workflowRows = stabilizeGroupIdentity(this.groupIdentityCache, rawWorkflowRows);
            // Retired rows are filtered here, not inside buildDispatchBuckets
            // (kept a pure, ViewModel-independent function) — see
            // filterRetired's doc comment for the un-retire-on-new-activity
            // mechanism (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6).
            const agentToolRows = filterRetired(rawAgentToolRows, retired, (s) => subagentRowKey(s.agent_id), (s) => s.last_event_at);
            // workflowRetireSignal, NOT w.lastEventAt — see that function's
            // doc comment (reagent P1 on PR #2677).
            const visibleWorkflowRows = filterRetired(workflowRows, retired, (w) => w.dispatchId, (w) => workflowRetireSignal(w));
            const shellRows = buildShellRows(shells, blockId);
            const cronRows = buildCronRows(crons, blockId);
            // Nested only under solo subagent rows that are actually shown;
            // anything else stays visible at the agent level.
            const shownSubagentToolUseIds = new Set(
                agentToolRows.map((s) => s.tool_use_id).filter((id): id is string => !!id)
            );
            const backgroundRows = groupBackgroundTasks(backgroundTasks, blockId, shownSubagentToolUseIds);
            const progress = progressByBlock.get(blockId);
            // Same "only while running" rule as `currentTool` below: a stopped
            // pane's last in-flight tool is stale.
            const liveTool = agentStatus === "running" ? (progress?.currentTool ?? null) : null;
            const line = resolveSwarmLine({
                meta: block?.meta as Record<string, unknown> | undefined,
                status: agentStatus,
                currentTool: liveTool,
                contextTokens,
            });
            return {
                blockId,
                agentName,
                agentProvider,
                activitySummary,
                line,
                contextTokens,
                agentStatus,
                agentToolRows,
                workflowRows: visibleWorkflowRows,
                shellRows,
                cronRows,
                todoRows: progress?.todos ?? [],
                todosTruncated: progress?.todosTruncated ?? 0,
                todosPartial: progress?.todosPartial ?? false,
                // Only meaningful while the agent is actually running — a
                // stopped pane's last in-flight tool is stale, and showing it
                // would read as "still doing this" (the payload is push-only,
                // so nothing else clears it on stop).
                currentTool: agentStatus === "running" ? (progress?.currentTool ?? null) : null,
                backgroundTasks: backgroundRows,
            };
        });

        // Prune once per full pass (not per-block — see stabilizeGroupIdentity).
        pruneGroupIdentityCache(this.groupIdentityCache, liveGroupKeys);

        return nodes;
    }

    dispose(): void {
        for (const unsub of [...this.unsubs, ...this.blockUnsubs]) unsub();
        this.unsubs = [];
        this.blockUnsubs = [];
        if (this.loadSubagentsDebounceTimer !== undefined) {
            clearTimeout(this.loadSubagentsDebounceTimer);
            this.loadSubagentsDebounceTimer = undefined;
        }
        if (this.loadShellsDebounceTimer !== undefined) {
            clearTimeout(this.loadShellsDebounceTimer);
            this.loadShellsDebounceTimer = undefined;
        }
        if (this.loadCronsDebounceTimer !== undefined) {
            clearTimeout(this.loadCronsDebounceTimer);
            this.loadCronsDebounceTimer = undefined;
        }
        if (this.loadBackgroundTasksDebounceTimer !== undefined) {
            clearTimeout(this.loadBackgroundTasksDebounceTimer);
            this.loadBackgroundTasksDebounceTimer = undefined;
        }
        if (this.trackedBlocksPollTimer !== undefined) {
            clearInterval(this.trackedBlocksPollTimer);
            this.trackedBlocksPollTimer = undefined;
        }
        for (const detail of this.dispatchDetailCache.values()) detail.dispose();
        this.dispatchDetailCache.clear();
        this.groupIdentityCache.clear();
        for (const timer of this.countdownTimers.values()) clearTimeout(timer);
        this.countdownTimers.clear();
    }
}
