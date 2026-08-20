// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { WOS } from "@/app/store/global";
import { callBackendService } from "@/store/wos";
import { BlockService } from "@/app/store/services";
import { readActivitySummary } from "@/app/store/activitySummary";
import { createSignal, type Accessor, type Setter } from "solid-js";

// ── Types ────────────────────────────────────────────────────────────────

// SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17: the backend now models
// two levels — `AgentDispatch` (one per Agent-tool-or-Workflow-tool call)
// containing N `SubAgent` members. The TS interface here keeps the
// `ActiveSubagent` name (unlike the Rust `SubagentInfo`→`SubAgent` rename)
// to limit churn across this file/swarm-view.tsx — it maps 1:1 to the
// backend's `SubAgent` either way.
export interface ActiveSubagent {
    agent_id: string;
    slug: string;
    parent_agent: string;
    parent_block_id: string;
    session_id: string;
    /** `"abandoned"` (SubAgentStatus::Abandoned, Rust) — the parent block's
     *  turn ended without a Result line ever appearing for this subagent
     *  (crashed, killed, or interrupted by an app/srv restart). Distinct
     *  from `"completed"`: it didn't finish, it was cut off. Set by the
     *  backend's `reconcile_stale_subagents`, currently only on pane
     *  reopen/backfill. */
    status: "active" | "completed" | "abandoned";
    /** Unix ms when this subagent was first observed — set once, immutable.
     *  Distinct from `last_event_at`, which advances on every journal read. */
    spawned_at: number;
    last_event_at: number;
    event_count: number;
    model: string | null;
    /** Always present now (was `workflow_id: string | null`) — every
     *  subagent has a real dispatch container. A Workflow-tool member
     *  carries the run's own id (`"wf_<id>"`); a solo Task-tool call gets a
     *  synthesized `"solo:<agent_id>"` (Rust's `solo_dispatch_id`). Use
     *  `dispatch_id.startsWith("solo:")` to tell them apart client-side. */
    dispatch_id: string;
    // Concise Haiku-generated name (SubAgent.display_name, Rust). Null
    // until a client expands this subagent's row for the first time — see
    // `subagent.GenerateName` / the `subagent:named` event below.
    display_name: string | null;
}

/**
 * Mirrors the backend's `ShellSummary` (`shell_node.rs`) — one currently-
 * RUNNING background shell the agent kicked off via the Shell MCP tool.
 * Fetched via `shell.ListActive` (unfiltered, like `ActiveSubagent` above);
 * `buildTree()` groups by `block_id` client-side. Exited shells never
 * appear here — Phase 1's scope is "what's happening now," not history.
 * See SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20.
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
 * Mirrors the backend's `CronSummary` (`server/cron.rs`) — one persistent
 * cron job whose `created_by` agent currently resolves to a live block.
 * Fetched via `cron.ListActive` (unfiltered, like `ActiveShell` above);
 * `buildTree()` groups by `block_id` client-side. Unlike Shell, a job stays
 * visible whether `enabled` or paused — Phase 2's scope is "does this agent
 * have a recurring job", not just "is it firing right now". See
 * SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2.
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
}

/**
 * Mirrors the backend's `AgentDispatch` (one per Agent-tool-or-Workflow-tool
 * call). Only Workflow-kind dispatches get their own tree row
 * (`WorkflowDispatch`, below) — a Solo-kind dispatch's one member renders as
 * a plain `ActiveSubagent` row directly, no wrapper (SPEC §5/§7).
 */
export interface AgentDispatch {
    dispatch_id: string;
    kind: "solo" | "workflow";
    parent_agent: string;
    parent_block_id: string;
    session_id: string;
    member_count: number;
    members_done: number;
    /** `"abandoned"` (DispatchStatus::Abandoned, Rust) added
     *  SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2 —
     *  every member is Completed|Abandoned and at least one is Abandoned
     *  (the parent block's turn ended before this dispatch finished). Both
     *  `"completed"` and `"abandoned"` are terminal for UI purposes — see
     *  `WorkflowDispatch.status`'s derivation below, which folds both into
     *  `"retired"`. */
    status: "running" | "completed" | "abandoned";
    last_event_at: number;
    /** For a Workflow-kind dispatch: a concise Haiku-generated name, resolved
     *  EAGERLY (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A)
     *  the first time its first member is observed live — not on-click like
     *  `ActiveSubagent.display_name`. `null` until resolved.
     *
     *  For a Solo-kind dispatch: mirrors that one member's own
     *  `display_name` directly (`subagent_watcher.rs`'s `solo_dispatch()`) —
     *  there's no separate dispatch-level naming call for Solo, so this is
     *  `null` until that member's `display_name` itself resolves, same
     *  timing as `ActiveSubagent.display_name`, not always `null`. */
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
    | { type: "tool_use"; name: string; input_summary: string }
    | { type: "tool_result"; is_error: boolean; preview: string }
    | { type: "progress"; output: string }
    | { type: "result"; content: string };

/**
 * One row for a Workflow-kind `AgentDispatch` — SPEC §7: never one row per
 * member, regardless of member count (a single workflow run can spawn
 * hundreds to low-thousands of members; see
 * docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md, which found
 * 1,030+ in one run). Deliberately does NOT carry the member list — a
 * dispatch this large can't hold its members' event histories in the tree
 * atom without reintroducing the same unbounded-volume problem this
 * redesign exists to fix. Expanding this row shows a live concatenated
 * activity feed instead (`createDispatchDetail`), not nested member rows.
 */
export interface WorkflowDispatch {
    kind: "workflowDispatch";
    dispatchId: string;
    /** Prefers the backend's eagerly-resolved `AgentDispatch.dispatch_name`
     *  (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A) —
     *  falls back to a member's `slug`, then the raw `dispatchId`, only for
     *  the brief window before the eager Haiku call resolves (or if it
     *  never does — see that spec's naming-failure fallback notes). */
    name: string;
    memberCount: number;
    membersDone: number;
    /** "active" if any member is still active; "retired" once every member
     *  has completed. Derived from AgentDispatch.status ("running" |
     *  "completed") — different vocabulary, same meaning as the rest of
     *  this file's group rows. */
    status: "active" | "retired";
    lastEventAt: number;
}

export interface AgentTreeNode {
    blockId: string | null;
    agentName: string;
    agentProvider: string | null;
    activitySummary: string | null;
    contextTokens: number | null;
    agentStatus: "running" | "idle";
    /** One row per Agent-tool (solo) dispatch — always a flat list, never
     *  grouped/collapsed by shared name (SPEC_SWARM_DISPATCH_NAMING_AND_
     *  ROW_MODEL_2026_07_19 §2, superseding the `NameGroup` slug-fallback
     *  approach from PR #2226: eager per-dispatch naming means there's no
     *  more "many rows share one uninformative label" problem to collapse
     *  away). Sorted by most recent activity. */
    agentToolRows: ActiveSubagent[];
    /** One row per Workflow-tool dispatch, always its own row regardless of
     *  member count (SPEC §7, unchanged from the pre-existing design).
     *  Sorted by most recent activity. */
    workflowRows: WorkflowDispatch[];
    /** One row per currently-RUNNING background shell this agent started
     *  (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 1). Sorted
     *  newest-first, same convention as the other two buckets. */
    shellRows: ActiveShell[];
    /** One row per cron job this agent created (SPEC_SWARM_LONG_RUNNING_
     *  PROCESS_ROWS_2026_07_20 Phase 2), enabled or paused. Sorted
     *  newest-first by `last_fired` (nulls — never fired — last). */
    cronRows: ActiveCron[];
}

/**
 * Build one block's two fixed row buckets from its `AgentDispatch`es
 * (already block-filtered) and raw `SubAgent`s — always exactly these two
 * groupings, never data-driven collapsing by shared name (see
 * `AgentTreeNode.agentToolRows`'s doc comment for why that approach was
 * retired). Each bucket is independently sorted by recency.
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
                // Both "completed" and "abandoned" are terminal — an
                // abandoned dispatch (dead parent, never finished) must not
                // read as "active" just because it isn't "completed".
                status: d.status === "running" ? ("active" as const) : ("retired" as const),
                lastEventAt: d.last_event_at,
            };
        })
        .sort((a, b) => b.lastEventAt - a.lastEventAt);

    const solo = subagents.filter((s) => s.dispatch_id.startsWith("solo:"));

    // Fallback for a failed/lagging `ListDispatches` call: `loadDispatches()`
    // swallows RPC errors and leaves `dispatchesAtom` stale (see its call
    // site), so `dispatches` here can lag or miss entries `subagents` (a
    // separate fetch) already has. Without this, any workflow-kind subagent
    // whose dispatch has no matching row in `workflowRows` would vanish from
    // the tree entirely.
    //
    // Grouped into ONE synthesized placeholder `WorkflowDispatch` row per
    // orphaned `dispatch_id`, appended to `workflowRows` — never spread
    // individually into `agentToolRows`. Spreading them flat previously
    // turned a lagging Workflow dispatch's member files into N separate
    // Agent-Tool rows (up to member_count of them), breaking the "one row
    // per Workflow-tool call" invariant `WorkflowDispatch`'s own doc comment
    // establishes, and inflating the visible row count independent of the
    // naming-collision issue (already fixed, see `subagentDisplayLabel`).
    // See docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.1.
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
            // Best-effort lower bound — the real member_count isn't known
            // until ListDispatches catches up; this only reflects the
            // members ListActive has surfaced so far for this dispatch.
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
 * Build one block's Shell bucket rows (SPEC_SWARM_LONG_RUNNING_PROCESS_
 * ROWS_2026_07_20 Phase 1) — every currently-running shell whose
 * `block_id` matches, newest-first. Extracted as a pure function (matching
 * `buildDispatchBuckets` above) purely so it's directly unit-testable
 * without instantiating `SwarmViewModel`.
 */
export function buildShellRows(shells: ActiveShell[], blockId: string | null): ActiveShell[] {
    return shells
        .filter((s) => s.block_id === blockId)
        .sort((a, b) => b.started_at - a.started_at);
}

/**
 * Build one block's Cron bucket rows (SPEC_SWARM_LONG_RUNNING_PROCESS_
 * ROWS_2026_07_20 Phase 2) — every cron job whose `block_id` matches,
 * newest-last-fired-first; a job that has never fired sorts last (nulls
 * carry no recency signal). Extracted as a pure function for the same
 * reason as `buildShellRows` — directly unit-testable, no ViewModel needed.
 */
export function buildCronRows(crons: ActiveCron[], blockId: string | null): ActiveCron[] {
    return crons
        .filter((c) => c.block_id === blockId)
        .sort((a, b) => (b.last_fired ?? -1) - (a.last_fired ?? -1));
}

/**
 * Whether a blockId in `buildTree()`'s row list should actually render a
 * row — false only when the id's WOS oref has been definitively resolved
 * to nothing. This is distinct from "the block exists but `agentName`
 * hasn't propagated to its meta yet" (the case `buildTree()`'s own
 * `"Agent"` fallback string is for, unchanged since 2026-06-22) — that's a
 * real, transient loading state on a real block, and from "the fetch for
 * this oref just hasn't resolved yet" (`isLoading`) — `WOS.
 * getWaveObjectAtom` seeds a freshly-tracked oref with `{ value: null,
 * loading: true }` (`wos.ts:152-153`) until its async `GetObject` fetch
 * resolves, so a genuinely real, just-spawned block's row would otherwise
 * read identically to a phantom one on the very first `buildTree()` pass —
 * reagentx P1 on #2438, and very plausibly the explanation for the
 * separate "a live-spawned subagent wasn't observed in Swarm" symptom this
 * retro originally left as an open, unconfirmed question. While loading,
 * this returns `true` (render it, same as today's pre-existing behavior —
 * self-corrects once the fetch resolves) rather than guessing either way.
 *
 * A `null`/`undefined` block with `isLoading === false` means the fetch
 * completed and there is genuinely nothing there: most commonly a
 * subagent's `parent_block_id` (added to `buildTree()`'s row-id set as a
 * registration-ordering fallback — see that comment) that never resolved
 * to a fully-registered block, or a pruned block whose subagent record
 * outlived it (`prune_block` is triggered by BlockDeleted/TabDeleted/
 * WorkspaceDeleted; a block whose OWN registration never completed was
 * never "deleted," so nothing prunes it). Rendering a placeholder
 * `"Agent"` row for an id that structurally doesn't exist is worse than
 * not rendering it — indistinguishable from a real agent to the user, and
 * any dispatch grouped under it shows as an empty "No activity yet" row
 * beside it. Extracted as a pure predicate so it's directly unit-testable
 * without instantiating `SwarmViewModel` or mocking WOS, same rationale as
 * `buildShellRows`/`buildCronRows` above.
 * See RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06.md.
 */
export function hasRenderableBlock<T>(block: T | null | undefined, isLoading: boolean): boolean {
    if (isLoading) return true;
    return block != null;
}

/**
 * Rows eligible for the bulk "Clear completed" action — every terminal-
 * status (not currently `"active"`) row across every `AgentTreeNode`, as a
 * flat list of `(rowKey, lastEventAt)` pairs ready to hand to `retireRow`.
 * Extracted as a pure function for direct unit-testability, mirroring
 * `buildShellRows`/`buildCronRows` above. A row already retired doesn't
 * need separate exclusion here — `buildTree()` already filters retired
 * rows out of the `AgentTreeNode`s this is called with (`filterRetired`),
 * so nothing eligible for a fresh retire is ever double-counted.
 * See SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3.
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
            // WorkflowDispatch.status is "active" | "retired" — "retired"
            // here means "every member completed" (backend-derived), NOT
            // "the user manually retired this row." Naming collision with
            // this exact feature, not a bug — see that field's own doc
            // comment on the WorkflowDispatch interface above.
            if (w.status !== "active") {
                result.push({ rowKey: w.dispatchId, lastEventAt: w.lastEventAt });
            }
        }
    }
    return result;
}

/**
 * Compute the label `SubagentRow` shows for a subagent, in priority order:
 * `display_name` (Haiku-resolved — as of
 * SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 Phase A, resolved
 * EAGERLY at dispatch time for the common case, not just on-click) > `slug`
 * > a short prefix of `agent_id`.
 *
 * `slug` is NOT a per-subagent-unique identifier — it's read straight
 * through from whatever the Claude Code CLI happened to write into the
 * first line of the subagent's own JSONL file (`subagent_watcher.rs`'s
 * `read_jsonl_from_offset`), which in practice is that CLI's own
 * per-session/per-batch codename. A whole Task/Workflow-tool batch of
 * genuinely distinct, unrelated subagents legitimately shares one slug —
 * already established as an expected, non-buggy signature in
 * docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md Finding 3
 * ("one shared slug = one legitimate concurrent spawn") and relied on by
 * `buildDispatchBuckets`'s `WorkflowDispatch.name` derivation above as a
 * fallback for the brief window before eager naming resolves.
 *
 * Falling back to a bare `slug` as a ROW LABEL, though, silently assumed
 * the opposite — that it WAS subagent-unique — so every member of such a
 * batch rendered as a visually identical `SubagentRow` until its own
 * `display_name` resolved. Live-reproduced in task #44: 17 structurally
 * distinct `agent_id`s, one shared literal slug, spawned within ~50ms of
 * each other under one parent, rendered as 17 apparently-duplicate rows —
 * not a slug-generation bug or a spawn-dedup bug (all 17 were genuinely
 * separate, correctly-ungrouped subagents; `dispatch_id`/`display_name`
 * grouping never entered the picture since neither had resolved yet).
 * Appending a short, always-unique `agent_id` suffix keeps same-slug
 * siblings visually distinct without claiming a uniqueness `slug` never
 * had — the same `agent_id.substring(0, 7)` disambiguator the now-retired
 * `SubagentDetailPane` used to show as a separate meta chip, for the same
 * reason.
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
 * Merge a freshly-fetched subagent list into the previous one, reusing the
 * OLD object reference for any entry whose fields are unchanged. `ListActive`
 * returns a brand-new JSON-deserialized array on every call — without this,
 * every subagent (not just the one that actually changed) gets a fresh
 * object identity on every spawn/completed refresh, and SolidJS's `<For>`
 * (which diffs list items by reference, not value) tears down and remounts
 * every row in the tree on every refresh, silently collapsing any row a
 * user has expanded. See
 * docs/specs/REPORT_SWARM_SUBAGENT_DETAIL_UX_ANALYSIS_2026_07_07.md.
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

/** Content-equality check for a `WorkflowDispatch` wrapper — used by
 *  `stabilizeGroupIdentity` to decide whether to reuse the old object
 *  reference or accept the freshly-built one. */
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

/** Cache key for a `WorkflowDispatch` — namespaced (`wf:`) even though only
 *  one kind uses this cache now, kept from the pre-two-bucket design so
 *  `expandedIdsAtom`'s ids stay stable across the change (a `NameGroup`'s
 *  `name:`-namespaced keys no longer exist — see the two-bucket redesign,
 *  SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19). */
export function groupCacheKey(child: WorkflowDispatch): string {
    return `wf:${child.dispatchId}`;
}

/** Row identity for an Agent Tool row — the same `agent:${agent_id}` key
 *  `swarm-view.tsx`'s `SubagentRow` already computes locally for expand-
 *  state/`DispatchActivityFeed` identity (SPEC_SWARM_DISPATCH_NAMING_AND_
 *  ROW_MODEL_2026_07_19 §4, decoupled from `dispatch_id` in the reagent
 *  P1 fix on #2232 — see `getDispatchDetail`'s doc comment). Exported so
 *  `SwarmViewModel`'s retire filter (§ below) and the row's own Retire
 *  button compute the identical key rather than two independently-written
 *  string templates drifting apart. */
export function subagentRowKey(agentId: string): string {
    return `agent:${agentId}`;
}

/**
 * Drop any row whose key is in `retired` AND whose own `lastEventAt` still
 * matches the snapshot taken at retire time — genuinely new activity for
 * that same key (a later `lastEventAt`) makes the row visible again
 * automatically, without a separate un-retire action
 * (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6). Shared by
 * both `AgentTreeNode` buckets in `buildTree()` — parameterized over `T`
 * rather than duplicated per row type.
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
 * Drop `retired` entries whose key is no longer in `liveKeys` at all — a row
 * gone from live state entirely (block closed, dispatch pruned — #2233) can
 * never un-retire itself (`filterRetired`'s lastEventAt-snapshot comparison
 * has nothing left to compare against), so keeping its retired entry around
 * is pure unbounded growth for the life of the session (reagent P2 on
 * #2235). A row still genuinely live keeps its entry regardless of this
 * pass — only entries for keys absent from `liveKeys` are dropped. Returns
 * `retired` unchanged (same reference) if nothing was actually dropped, so
 * callers using this in a signal setter don't trigger a no-op update.
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

// ── Retired-row persistence (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3) ──
//
// `_retiredRowKeys` used to be purely in-memory/ephemeral (reset on every
// reload/restart) — the only "dismiss" mechanism for a dead-session row,
// forgotten the moment the pane remounted, which is a direct contributor to
// the "48 rows for a couple of real calls" symptom that spec root-causes.
// Persisted to localStorage, local-machine scope (not synced/cross-device —
// a deliberate choice, see that spec's §6 decisions), mirroring the
// existing `toolchain-view.tsx` `loadWidgetPorts`/`saveWidgetPort` pattern
// (namespaced key, JSON, defensive try/catch — localStorage access can
// throw in some embedding contexts).

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
        // best-effort — a full localStorage or a context where it throws
        // just means this session's retires don't survive reload, same as
        // the old always-ephemeral behavior.
    }
}

/**
 * Stabilize `WorkflowDispatch` wrapper identity across `buildTree()` calls,
 * mirroring `mergeSubagentsPreservingIdentity` one level up.
 * `buildDispatchBuckets` is a pure function — it unconditionally builds
 * brand-new wrapper objects per call, even when nothing about a given
 * dispatch actually changed. Left alone, that fresh wrapper still remounts
 * `WorkflowDispatchRow` — and everything nested inside an expanded one — on
 * every unrelated tree recompute, which for a large workflow dispatch
 * defeats the very remount fix `expandedIdsAtom`/`getDispatchDetail` were
 * meant to provide.
 *
 * `cache` is a `Map<groupCacheKey, dispatch>` the caller keeps around (one
 * per `SwarmViewModel`), shared and called once per BLOCK within one
 * `buildTree()` pass — this function only reuses-or-replaces into `cache`,
 * it never prunes it (pruning here, scoped to one block's rows, would evict
 * entries a DIFFERENT block's call just wrote). Callers doing a full
 * multi-block tree rebuild should prune the cache once afterward against
 * every key actually produced across the whole tree — see
 * `pruneGroupIdentityCache`.
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
 * Merge freshly-arrived entries into `prev`, de-duplicating by
 * `dispatchActivityEntryKey`, re-sorting by timestamp, and capping to
 * `MAX_DISPATCH_FEED_ENTRIES`. Exported (pure, no signal access) so the
 * de-dup behavior itself is unit-testable — see swarm-model.test.ts.
 *
 * De-dup matters because a solo dispatch's `GetHistory` backfill and the
 * live `dispatch:activity` broadcast race independently: an event the
 * backend already flushed into `pending_activity` before `GetHistory`
 * resolves is present in both, so blindly concatenating double-counts it
 * (reagent P1 on #2232).
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
 * Expanding a row (`WorkflowDispatchRow`, or an Agent Tool row —
 * SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4) shows this: every
 * member's new events, merged into one chronological, member-tagged stream,
 * fed by the backend's coalesced `dispatch:activity` broadcast
 * (`subagent_watcher.rs`'s `flush_pending_dispatch_activity` — as of Phase A
 * this fires for solo `dispatch_id`s too, dual-emitted alongside the
 * existing immediate `subagent:activity`).
 *
 * Live activity is LIVE-ONLY for a Workflow dispatch — SPEC §9.4 leaves the
 * backfill/pagination question for a large multi-hundred-member workflow
 * explicitly open (eagerly fetching + merging thousands of prior events on
 * every expand would reintroduce the exact request/render volume problem
 * this redesign exists to fix), and there is no bulk-backfill RPC for a
 * whole workflow's history today. A single subagent's history, though, is
 * bounded (one member) and an RPC for it already exists — `subagent.
 * GetHistory` — so backfilling it here closes the one real regression this
 * unification would otherwise introduce (the retired `SubagentDetailPane`/
 * `createSubagentDetail` DID backfill via that same RPC).
 *
 * `backfillAgentId` (optional): scopes BOTH the `GetHistory` backfill AND
 * the live `dispatch:activity` subscription down to this one agent's own
 * events. Callers rendering a single-subagent row (any Agent Tool row,
 * including an orphaned workflow member — see `getDispatchDetail`'s doc
 * comment) should pass the subagent's own `agent_id` explicitly rather than
 * relying on `dispatchId` happening to start with `"solo:"` — that prefix
 * alone doesn't cover the orphaned-member case, AND an orphaned member's
 * `dispatchId` can be shared with sibling rows, so without this filter a
 * single-subagent row would render every sibling's live events too. Falls
 * back to parsing the `"solo:"` prefix when omitted, for callers with no
 * `ActiveSubagent` in scope. Omitted entirely for a genuine multi-member
 * `WorkflowDispatchRow`, which legitimately wants every member's events.
 */
export function createDispatchDetail(dispatchId: string, backfillAgentId?: string): DispatchDetail {
    const [entries, setEntries] = createSignal<DispatchActivityEntry[]>([]);

    const mergeIncoming = (incoming: DispatchActivityEntry[]): void => {
        if (incoming.length === 0) return;
        setEntries((prev) => mergeDispatchActivityEntries(prev, incoming));
    };

    const unsub = waveEventSubscribe({
        eventType: "dispatch:activity",
        handler: (event: WaveEvent) => {
            const data = event?.data as any;
            if (data?.dispatchId !== dispatchId) return;
            const members = (data?.members as { agentId: string; events: SubagentEvent[] }[]) ?? [];
            const relevant = backfillAgentId ? members.filter((m) => m.agentId === backfillAgentId) : members;
            mergeIncoming(relevant.flatMap((m) => (m.events ?? []).map((evt) => ({ agentId: m.agentId, event: evt }))));
        },
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
 * `turn_active` is turn-precise (backed by the health monitor wired to the
 * NDJSON stream) but only meaningful for persistent/ACP agent controllers —
 * `is_agent_pane` is the discriminator, since `turn_active: false` and "this
 * controller never populates turn_active" are indistinguishable on the wire
 * (the Rust struct omits `false` fields — `skip_serializing_if = "is_false"`).
 * For `is_agent_pane` panes, trust `turn_active` alone: `shellprocstatus`
 * stays `"running"` for a persistent agent's entire process lifetime,
 * idle-between-turns included, so OR-ing it back in would misrepresent an
 * idle agent as "working" again — the exact bug this field exists to fix.
 * For everything else (shell/PTY panes with no turn concept), fall back to
 * `shellprocstatus` as before. See
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

export class SwarmViewModel implements ViewModel {
    viewType = "swarm";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "diagram-project";
    viewName: Accessor<string> = () => "Swarm";
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // set by barrel
    }

    private _subagents = createSignal<ActiveSubagent[]>([]);
    subagentsAtom: Accessor<ActiveSubagent[]> = this._subagents[0];
    private setSubagents: Setter<ActiveSubagent[]> = this._subagents[1];

    // Active background shells (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20
    // Phase 1) — fetched separately via shell.ListActive; buildTree() groups
    // by block_id the same way it does for subagents/dispatches above.
    private _shells = createSignal<ActiveShell[]>([]);
    shellsAtom: Accessor<ActiveShell[]> = this._shells[0];
    private setShells: Setter<ActiveShell[]> = this._shells[1];

    // Cron jobs (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2) —
    // fetched separately via cron.ListActive; buildTree() groups by
    // block_id the same way it does for shells above.
    private _crons = createSignal<ActiveCron[]>([]);
    cronsAtom: Accessor<ActiveCron[]> = this._crons[0];
    private setCrons: Setter<ActiveCron[]> = this._crons[1];

    // AgentDispatches (SPEC §5) — one per Agent-tool-or-Workflow-tool call.
    // Fetched separately from subagentsAtom via subagent.ListDispatches;
    // buildTree() cross-references the two by dispatch_id/parent_block_id.
    private _dispatches = createSignal<AgentDispatch[]>([]);
    dispatchesAtom: Accessor<AgentDispatch[]> = this._dispatches[0];
    private setDispatches: Setter<AgentDispatch[]> = this._dispatches[1];

    // Map of blockId → "running" | "idle" — updated by controllerstatus events
    private _agentStatuses = createSignal<Map<string, "running" | "idle">>(new Map());
    agentStatusesAtom: Accessor<Map<string, "running" | "idle">> = this._agentStatuses[0];
    private setAgentStatuses: Setter<Map<string, "running" | "idle">> = this._agentStatuses[1];

    // Ordered list of tracked block IDs (preserves server-side ordering)
    private _trackedBlockIds = createSignal<string[]>([]);
    trackedBlockIdsAtom: Accessor<string[]> = this._trackedBlockIds[0];
    private setTrackedBlockIds: Setter<string[]> = this._trackedBlockIds[1];

    private _loading = createSignal<boolean>(true);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading: Setter<boolean> = this._loading[1];

    // Rows the user has expanded — keyed by dispatchId (WorkflowDispatchRow
    // and, since the two-bucket redesign, SubagentRow too — both use
    // toggleDispatchExpanded now). Lives here, not as row-local component
    // state: `tree()` recomputes on every trackedBlockIds/subagents/
    // agentStatuses change (agentStatuses updates on every controllerstatus
    // tick — very frequent during an active turn) and rebuilds fresh
    // WorkflowDispatch/AgentTreeNode wrapper objects every time regardless
    // of whether that row's own data changed, so `<For>`'s reference-diffing
    // remounts row components far more often than "the user actually
    // changed something." Local expand state would silently collapse on the
    // very next unrelated status tick; keying by a stable string id here
    // survives that churn.
    private _expandedIds = createSignal<Set<string>>(new Set());
    expandedIdsAtom: Accessor<Set<string>> = this._expandedIds[0];
    private setExpandedIds: Setter<Set<string>> = this._expandedIds[1];

    // Top-level agent rows now share the SAME default (collapsed) and
    // polarity (absent-means-collapsed) as the group/subagent _expandedIds
    // set above — this tracks which agent blockIds have been explicitly
    // expanded, so the empty default set means every agent starts
    // collapsed. Kept as its own signal rather than folded into
    // _expandedIds since agent blockIds and subagent/workflow row keys are
    // different id spaces that could theoretically collide. Same
    // ViewModel-residency rationale as _expandedIds: tree() rebuilds
    // wrapper objects on every status tick, so row-local state would
    // silently reset on unrelated refreshes.
    private _expandedAgentIds = createSignal<Set<string>>(new Set());
    expandedAgentIdsAtom: Accessor<Set<string>> = this._expandedAgentIds[0];
    private setExpandedAgentIds: Setter<Set<string>> = this._expandedAgentIds[1];

    // Rows the user has retired (dismissed) — client-local (no backend
    // write), but persisted to localStorage as of
    // SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3, so a
    // dismissal survives reload/restart instead of resurfacing every time
    // (previously ephemeral, same as memory-pressure-banner.tsx's
    // dismissedAt — that comparison no longer applies). Maps rowKey -> the
    // row's own lastEventAt AT THE MOMENT it was retired, not just a bare
    // membership set: this is what lets a row un-retire itself automatically
    // the moment genuinely new activity arrives for that same key
    // (buildTree()'s filter only suppresses a row whose CURRENT lastEventAt
    // still matches the snapshot) instead of requiring an explicit
    // un-retire action — see SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6.
    private _retiredRowKeys = createSignal<Map<string, number>>(loadRetiredRowKeysFromStorage());
    retiredRowKeysAtom: Accessor<Map<string, number>> = this._retiredRowKeys[0];
    private rawSetRetiredRowKeys: Setter<Map<string, number>> = this._retiredRowKeys[1];
    /** Every write to `_retiredRowKeys` must go through this — persists to
     *  localStorage alongside the in-memory update, so a raw
     *  `this.rawSetRetiredRowKeys` call is a bug (nothing enforces this at
     *  the type level, but every call site in this file goes through here). */
    private setRetiredRowKeys(updater: (prev: Map<string, number>) => Map<string, number>): void {
        this.rawSetRetiredRowKeys((prev) => {
            const next = updater(prev);
            saveRetiredRowKeysToStorage(next);
            return next;
        });
    }

    // Auto-linger countdown on a row's first clean-terminal appearance
    // (SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06) — same rowKey space
    // as _retiredRowKeys above (subagentRowKey(agent_id) or a Workflow
    // dispatchId). `lastEventAt` is the row's own value at arm/last-reset
    // time, same snapshot-comparison idea `_retiredRowKeys` already uses so
    // genuinely new activity un-arms rather than lets a stale countdown
    // fire. `startedAt` (Date.now()) is what the visible remaining-seconds
    // display renders against while live. `pausedAt`: null while counting
    // down; set to Date.now() by pauseCountdown() so the DISPLAYED number
    // actually freezes (countdownSecondsRemaining computes elapsed against
    // pausedAt instead of the live clock while it's non-null) — merely
    // clearing the pending timer, as an earlier draft of this did, stops
    // the auto-retire from firing but does nothing to the wall-clock-
    // derived display, which kept counting to and clamping at 0 while
    // "paused" (reagentx P1 on #2440). resumeCountdown() clears pausedAt
    // and resets startedAt for a fresh 60s window.
    private _countdownState = createSignal<Map<string, CountdownEntry>>(new Map());
    countdownStateAtom: Accessor<Map<string, CountdownEntry>> = this._countdownState[0];
    private setCountdownState: Setter<Map<string, CountdownEntry>> = this._countdownState[1];
    private countdownTimers = new Map<string, ReturnType<typeof setTimeout>>();
    // The visible "Nn s" tick source lives in the view (useTick(1000),
    // frontend/app/hook/useTick.ts) — it's already the established,
    // ref-counted, auto-cleanup pattern this exact file uses elsewhere for
    // relative-time displays, so the ViewModel only needs to own the
    // start/lastEventAt state, not a second timer driving re-renders.

    // One DispatchDetail (concatenated activity feed) per currently-expanded
    // row — Agent Tool (solo) rows and Workflow rows alike, unified in
    // SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4 (retired the
    // separate per-subagent SubagentDetail/detailCache this used to be split
    // from). Created lazily on first expand: if this fetch+subscribe
    // lifecycle lived inside the row component instead, every incidental
    // remount (see expandedIds above) would refetch/resubscribe from
    // scratch — potentially several times a second while a row is open
    // during an active turn.
    private dispatchDetailCache = new Map<string, DispatchDetail>();

    // Persisted across buildTree() calls so stabilizeGroupIdentity can reuse
    // the same WorkflowDispatch wrapper object when a dispatch's own content
    // is unchanged — without this, expandedIds/dispatchDetailCache above
    // still don't help a row NESTED inside an expanded one, since <For>
    // remounts the whole WorkflowDispatchRow subtree whenever the wrapper's
    // own reference changes, which is every buildTree() call otherwise.
    private groupIdentityCache = new Map<string, WorkflowDispatch>();

    private unsubs: (() => void)[] = [];
    // Per-block controllerstatus unsubs — cleaned up when block list refreshes
    private blockUnsubs: (() => void)[] = [];

    // Backend broadcasts one subagent:spawned/subagent:completed event per
    // subagent file (see subagent_watcher.rs's process_jsonl_change) — a
    // backfill scan on pane reopen can fire dozens of these in a burst.
    // Debounce the resulting loadSubagents() RPC here instead of batching the
    // broadcasts themselves, since activity/subagent-source.ts (a different
    // consumer of the same events, driving the ActivityDock) needs one event
    // per subagent.
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

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        void this.loadAll();

        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: () => this.scheduleLoadSubagents(),
        });
        if (unsubSpawned) this.unsubs.push(unsubSpawned);

        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: () => this.scheduleLoadSubagents(),
        });
        if (unsubCompleted) this.unsubs.push(unsubCompleted);

        // One or more subagents just reconciled active -> abandoned, either
        // live (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20
        // Phase A, #2234 — the instant their parent's turn ends) or at
        // reopen — reload so the row's display status
        // (subagentDisplayStatus) updates without waiting for an unrelated
        // event. The backend broadcast for this event lives in #2234, a
        // separate PR from this one (Phase B) — if this merges first, the
        // listener is simply inert until #2234 lands (subagent:abandoned
        // is never emitted yet), not broken; #2234's own PR description
        // covers the same sequencing from the backend side.
        const unsubAbandoned = waveEventSubscribe({
            eventType: "subagent:abandoned",
            handler: () => this.scheduleLoadSubagents(),
        });
        if (unsubAbandoned) this.unsubs.push(unsubAbandoned);

        // dispatch:updated fires for both Solo and Workflow kinds (SPEC §5) —
        // a member count/status change on either warrants the same reload
        // used for subagent spawn/completion (they're closely correlated:
        // a workflow member spawning/completing IS a dispatch update).
        const unsubDispatchUpdated = waveEventSubscribe({
            eventType: "dispatch:updated",
            handler: () => this.scheduleLoadSubagents(),
        });
        if (unsubDispatchUpdated) this.unsubs.push(unsubDispatchUpdated);

        // A closed block's subagents/dispatches were just pruned server-side
        // (SubagentWatcher::prune_block, backstopped against BlockDeleted/
        // TabDeleted/WorkspaceDeleted) — reload so the ghost row this used
        // to leave behind (visible until srv restart) disappears promptly.
        const unsubBlockPruned = waveEventSubscribe({
            eventType: "subagent:block_pruned",
            handler: () => this.scheduleLoadSubagents(),
        });
        if (unsubBlockPruned) this.unsubs.push(unsubBlockPruned);

        // Shell bucket (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 1).
        // shell_node_create — a new shell appeared, reload the active list.
        const unsubShellCreate = waveEventSubscribe({
            eventType: "shell_node_create",
            handler: () => this.scheduleLoadShells(),
        });
        if (unsubShellCreate) this.unsubs.push(unsubShellCreate);

        // shell_chunk fires per output line too — only reload on the
        // terminal "exit" op, not every line of stdout/stderr (that would
        // re-fetch the whole active-shells list on every chunk).
        const unsubShellChunk = waveEventSubscribe({
            eventType: "shell_chunk",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (data?.op === "exit") this.scheduleLoadShells();
            },
        });
        if (unsubShellChunk) this.unsubs.push(unsubShellChunk);

        // Cron bucket (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2).
        // Payload-free — any create/fire/pause/resume/delete just triggers a
        // full reload of the (already-cheap, unfiltered) active list.
        const unsubCronChanged = waveEventSubscribe({
            eventType: "cron_changed",
            handler: () => this.scheduleLoadCrons(),
        });
        if (unsubCronChanged) this.unsubs.push(unsubCronChanged);

        // Patch display_name in place (not a full loadSubagents() reload) so
        // every client watching this session picks up a generated name —
        // not just the one whose expand click triggered subagent.GenerateName.
        const unsubNamed = waveEventSubscribe({
            eventType: "subagent:named",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                const agentId = data?.agentId;
                const displayName = data?.displayName;
                if (!agentId || !displayName) return;
                this.setSubagents((prev) =>
                    prev.map((s) => (s.agent_id === agentId ? { ...s, display_name: displayName } : s))
                );
            },
        });
        if (unsubNamed) this.unsubs.push(unsubNamed);

        // When process trackers change, refresh the block list
        const unsubProcAdded = waveEventSubscribe({
            eventType: "agent:process-added",
            handler: () => void this.loadTrackedBlocks(),
        });
        if (unsubProcAdded) this.unsubs.push(unsubProcAdded);

        const unsubProcExited = waveEventSubscribe({
            eventType: "agent:process-exited",
            handler: () => void this.loadTrackedBlocks(),
        });
        if (unsubProcExited) this.unsubs.push(unsubProcExited);

        // When a reactive-handler agent (Claude Code pane) registers or
        // unregisters, refresh the block list. These events are distinct from
        // agent:process-added / agent:process-exited so useProcessCount doesn't
        // treat reactive registrations as phantom OS processes.
        const unsubReactiveReg = waveEventSubscribe({
            eventType: "agent:reactive-registered",
            handler: () => void this.loadTrackedBlocks(),
        });
        if (unsubReactiveReg) this.unsubs.push(unsubReactiveReg);

        const unsubReactiveUnreg = waveEventSubscribe({
            eventType: "agent:reactive-unregistered",
            handler: () => void this.loadTrackedBlocks(),
        });
        if (unsubReactiveUnreg) this.unsubs.push(unsubReactiveUnreg);

        // term:osc_title / term:ambient_summary meta changes — force re-read
        // of block meta. The block atom in WOS updates reactively, so the
        // memo in the view already reacts; no explicit handler needed here
        // beyond the WOS atom.
    }

    loadAll = async (): Promise<void> => {
        this.setLoading(true);
        try {
            await Promise.all([
                this.loadTrackedBlocks(),
                this.loadSubagents(),
                this.loadDispatches(),
                this.loadShells(),
                this.loadCrons(),
            ]);
            this.pruneRetiredRowKeys();
            this.reconcileCountdowns();
        } finally {
            this.setLoading(false);
        }
    };

    loadTrackedBlocks = async (): Promise<void> => {
        try {
            const { block_ids } = await RpcApi.AgentTrackedBlocksCommand(TabRpcClient, {});
            const ids: string[] = block_ids ?? [];
            this.setTrackedBlockIds(ids);
            this.subscribeToBlockStatuses(ids);
        } catch {
            // silent — safe default is empty tree
        }
    };

    loadSubagents = async (): Promise<void> => {
        try {
            const result = await callBackendService("subagent", "ListActive", []);
            const list = (result as ActiveSubagent[]) ?? [];
            this.setSubagents((prev) => mergeSubagentsPreservingIdentity(prev, list));
        } catch {
            // silently ignore
        }
    };

    loadDispatches = async (): Promise<void> => {
        try {
            const result = await callBackendService("subagent", "ListDispatches", []);
            this.setDispatches((result as AgentDispatch[]) ?? []);
        } catch {
            // silently ignore
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

    // Coalesces a burst of subagent:spawned/subagent:completed/dispatch:
    // updated events (e.g. a backfill scan on pane reopen, or a large
    // workflow dispatch spawning many members at once) into a single
    // reload fired after the burst settles, instead of one RPC pair per
    // event.
    scheduleLoadSubagents = (): void => {
        if (this.loadSubagentsDebounceTimer !== undefined) {
            clearTimeout(this.loadSubagentsDebounceTimer);
        }
        this.loadSubagentsDebounceTimer = setTimeout(() => {
            this.loadSubagentsDebounceTimer = undefined;
            void Promise.all([this.loadSubagents(), this.loadDispatches()]).then(() => {
                this.pruneRetiredRowKeys();
                this.reconcileCountdowns();
            });
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

    /** Drop `_retiredRowKeys` entries for rows no longer present in live
     *  state at all (block closed, dispatch pruned — #2233) — those can
     *  never un-retire themselves (`filterRetired`'s lastEventAt-snapshot
     *  comparison has nothing left to compare against), so keeping them
     *  around is pure unbounded growth for the life of the session (reagent
     *  P2 on #2235). Called after `loadSubagents`/`loadDispatches` settle
     *  (a plain state update, not from inside `buildTree()`'s own read of
     *  `retiredRowKeysAtom`, which would be a reactive write-during-read). */
    private pruneRetiredRowKeys(): void {
        const liveKeys = new Set<string>();
        for (const s of this.subagentsAtom()) liveKeys.add(subagentRowKey(s.agent_id));
        for (const d of this.dispatchesAtom()) {
            if (d.kind === "workflow") liveKeys.add(d.dispatch_id);
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

    toggleAgentCollapsed(blockId: string): void {
        this.setExpandedAgentIds((prev) => {
            const next = new Set(prev);
            if (next.has(blockId)) next.delete(blockId);
            else next.add(blockId);
            return next;
        });
    }

    /** Retire (dismiss) a row — `rowKey` is `subagentRowKey(agent_id)` for
     *  an Agent Tool row or the raw `dispatchId` for a `WorkflowDispatchRow`
     *  (same keys `buildTree()`'s filter checks against). `lastEventAt` is
     *  the row's own value at the moment of retiring — see `_retiredRowKeys`
     *  for why this snapshot, not a bare membership set, is what makes a
     *  row un-retire itself automatically on new activity. Callers should
     *  only expose this for a terminal-status row (`"idle"`/`"interrupted"`)
     *  — nothing to dismiss on a row still genuinely `"working"`. Also
     *  short-circuits any pending auto-linger countdown for this row (a
     *  manual dismiss mid-countdown shouldn't leave an orphaned timer that
     *  fires later against an already-retired key — harmless since
     *  retireRow is idempotent, but pointless to leave running). */
    retireRow(rowKey: string, lastEventAt: number): void {
        this.setRetiredRowKeys((prev) => {
            const next = new Map(prev);
            next.set(rowKey, lastEventAt);
            return next;
        });
        this.clearCountdown(rowKey);
    }

    /** Bulk "Clear completed" — retires every currently-visible terminal-
     *  status row across every block in one action, instead of requiring a
     *  human to click Retire one row at a time on a large historical
     *  backlog. The persisted-retire fix (`_retiredRowKeys` now surviving
     *  reload) makes this genuinely useful — a bulk clear that reset itself
     *  every reload wouldn't have been worth adding.
     *  See SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.3. */
    retireAllCompleted(): void {
        for (const { rowKey, lastEventAt } of collectClearableRows(this.buildTree())) {
            this.retireRow(rowKey, lastEventAt);
        }
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

    /** Arm/clear countdowns against current subagent/dispatch state — called
     *  after loadSubagents()/loadDispatches() settle, same timing as
     *  pruneRetiredRowKeys(). In scope: agentToolRows/workflowRows only,
     *  per the spec's own "out of scope: shellRows/cronRows" note.
     *
     *  Critically, "agentToolRows" here means `buildDispatchBuckets()`'s
     *  actual `agentToolRows` output (solo + orphaned-workflow-member
     *  subagents) — NOT the raw, unfiltered `subagentsAtom()` list, which
     *  also contains every NORMAL (non-orphaned) member of a still-tracked
     *  Workflow dispatch. Those never get their own `SubagentRow` (SPEC §7
     *  — a Workflow dispatch this large can't hold thousands of members'
     *  rows, hence `WorkflowDispatchRow`'s aggregate-only display). Arming
     *  a countdown per completed member of, say, a 1,030-member workflow
     *  (a real scale documented elsewhere in this file) would reintroduce
     *  exactly the per-member `setTimeout`/state-entry cost this design
     *  otherwise avoids — AND permanently hide a member that later surfaces
     *  through the `orphanedWorkflowMembers` fallback (a stale/failed
     *  `ListDispatches` call): `retireRow` would already have marked its
     *  `subagentRowKey` retired against its (unchanged, since it already
     *  completed) `last_event_at`, so `filterRetired` suppresses it forever
     *  the moment it needs that exact fallback to stay visible during the
     *  lag (reagentx P1 on #2440, second pass). Reusing
     *  `buildDispatchBuckets` directly — rather than re-deriving its
     *  solo/orphaned logic here — keeps this arming scope byte-for-byte in
     *  sync with what actually renders, including if that logic changes.
     *
     *  A subagent's `status === "completed"` is exactly
     *  `subagentDisplayStatus()`'s (swarm-view.tsx) `"idle"` case regardless
     *  of parent status — `"active"`/`"abandoned"` never map to `"idle"` —
     *  so checking it directly here arms the same set that function's
     *  `"idle"` would, without duplicating its parent-status branching or
     *  importing across the model/view boundary. A `"completed"` Workflow
     *  dispatch is this bucket's equivalent terminal state. Neither bucket
     *  has a distinct dispatch-level "failed" status to exclude beyond
     *  what's already excluded by only matching `"completed"` — an
     *  `"abandoned"` subagent (interrupted) is never `"completed"`, so it's
     *  already outside `liveTerminal` without a separate check.
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
        for (const [rowKey, lastEventAt] of liveTerminal) {
            this.armCountdown(rowKey, lastEventAt);
        }
    }

    /** Every row's toggle. `rowKey` must be a per-ROW-unique identity, NOT
     *  necessarily the row's `dispatch_id` — a `WorkflowDispatchRow`'s own
     *  `dispatch_id` is always 1:1 with its row, but an Agent Tool row's
     *  `dispatch_id` is NOT: an orphaned workflow member (falling back into
     *  the Agent Tool bucket while `ListDispatches` is stale/lagging) shares
     *  its real `dispatch_id` with every sibling member still waiting on
     *  the same lag. Callers rendering a single-subagent row must pass a
     *  key derived from something per-row-unique instead (e.g.
     *  `agent:${sub.agent_id}`) — see `getDispatchDetail`'s doc comment.
     *  Tears down the cached DispatchDetail (and its WS subscriptions) on
     *  collapse, so a row a user opened once and moved on from doesn't keep
     *  a live subscription forever. Unified in
     *  SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19 §4 (was split
     *  into toggleSubagentExpanded/toggleDispatchExpanded). */
    toggleDispatchExpanded(rowKey: string): void {
        const wasExpanded = this.isExpanded(rowKey);
        this.toggleExpanded(rowKey);
        if (wasExpanded) {
            this.dispatchDetailCache.get(rowKey)?.dispose();
            this.dispatchDetailCache.delete(rowKey);
        }
    }

    /**
     * `rowKey`: cache identity for THIS row — must be unique per row (see
     * `toggleDispatchExpanded`'s doc comment for why this can't just be
     * `dispatchId` for an Agent Tool row). Two sibling rows sharing one
     * `dispatchId` but caching under distinct `rowKey`s previously each
     * subscribed to the SAME live `dispatch:activity` broadcast unfiltered,
     * so each would render every OTHER sibling's events too — fixed by
     * `dispatchId`: the real dispatch_id to subscribe to for live
     * `dispatch:activity` events (may legitimately be shared across sibling
     * rows — `createDispatchDetail` itself filters live events down to
     * `backfillAgentId` when one is given, so this is safe even then).
     *
     * `backfillAgentId`: pass the row's own single `ActiveSubagent.agent_id`
     * whenever the caller is rendering exactly one subagent's row (an Agent
     * Tool row, including an orphaned workflow member) — NOT just when
     * `dispatchId` happens to start with `"solo:"`, which silently dropped
     * an orphaned member's `GetHistory` backfill (reagent P1 on #2232).
     * Omit it for a genuine `WorkflowDispatchRow`, which represents many
     * members and has no one agent to backfill or filter to.
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

        for (const blockId of blockIds) {
            // Fetch current status — don't assume idle for already-running agents.
            void BlockService.GetControllerStatus(blockId)
                .then((rts) => {
                    const status = derivedRunningStatus(rts?.is_agent_pane, rts?.turn_active, rts?.shellprocstatus);
                    this.setAgentStatuses((prev) => {
                        const m = new Map(prev);
                        m.set(blockId, status);
                        return m;
                    });
                })
                .catch(() => {/* keep idle default */});

            const scope = WOS.makeORef("block", blockId);
            const unsub = waveEventSubscribe({
                eventType: WpsEvent.ControllerStatus,
                scope,
                handler: (ev) => {
                    const data = (ev as any)?.data;
                    const next = derivedRunningStatus(data?.is_agent_pane, data?.turn_active, data?.shellprocstatus);
                    this.setAgentStatuses((prev) => {
                        const m = new Map(prev);
                        m.set(blockId, next);
                        return m;
                    });
                },
            });
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
        const statuses = this.agentStatusesAtom();

        // Include parent block IDs from subagents as fallback for agent panes
        // that registered subagents before their own registration propagated.
        const parentIds = subagents.map((s) => s.parent_block_id).filter(Boolean);
        const allBlockIds = [...new Set([...blockIds, ...parentIds])];

        // Collected from the WorkflowDispatch rows actually produced below,
        // not derived from the raw dispatch list, so stabilizeGroupIdentity's
        // cache only ever retains keys for dispatches actually present this
        // pass — same "harmless either way, but this is the precise set"
        // rationale the pre-two-bucket design used for NameGroup keys.
        const liveGroupKeys = new Set<string>();
        const retired = this.retiredRowKeysAtom();
        const nodes = allBlockIds.flatMap((blockId) => {
            const blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
            const block = blockAtom();
            // isLoading distinguishes "this oref hasn't resolved yet" from
            // "this oref resolved to nothing" — both read as block == null,
            // but only the latter means the id is genuinely phantom.
            // getWaveObjectLoadingAtom returns `null` while loading, `false`
            // once GetObject has resolved either way (wos.ts:232-238).
            const isLoading = WOS.getWaveObjectLoadingAtom(`block:${blockId}`)() !== false;
            if (!hasRenderableBlock(block, isLoading)) return [];
            const agentName =
                (block?.meta?.["agentName"] as string | undefined)?.trim() ||
                "Agent";
            const agentProvider =
                (block?.meta?.["agentProvider"] as string | undefined)?.trim() || null;
            const activitySummary = readActivitySummary(block?.meta)?.trim() || null;
            const rawCtx = block?.meta?.["term:ctx-tokens"];
            const contextTokens = typeof rawCtx === "number" ? rawCtx : null;
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
            const visibleWorkflowRows = filterRetired(workflowRows, retired, (w) => w.dispatchId, (w) => w.lastEventAt);
            const shellRows = buildShellRows(shells, blockId);
            const cronRows = buildCronRows(crons, blockId);
            return {
                blockId,
                agentName,
                agentProvider,
                activitySummary,
                contextTokens,
                agentStatus,
                agentToolRows,
                workflowRows: visibleWorkflowRows,
                shellRows,
                cronRows,
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
        for (const detail of this.dispatchDetailCache.values()) detail.dispose();
        this.dispatchDetailCache.clear();
        this.groupIdentityCache.clear();
        for (const timer of this.countdownTimers.values()) clearTimeout(timer);
        this.countdownTimers.clear();
    }
}
