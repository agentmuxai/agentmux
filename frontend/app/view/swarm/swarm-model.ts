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
    status: "running" | "completed";
    last_event_at: number;
}

// ── Subagent event log (inline-expand detail) ───────────────────────────

export interface SubagentEvent {
    agent_id: string;
    event_type: SubagentEventType;
    timestamp: number;
}

export type SubagentEventType =
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
    /** Derived client-side from a member's slug (SPEC keeps this — the
     *  backend still has no separate dispatch-name concept). */
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

/**
 * A group of solo-dispatch subagents (no Workflow-tool run — a Solo
 * `AgentDispatch` each) that independently earned the same Haiku-generated
 * `display_name`. A user repeatedly spawning similar tasks (e.g. "review this
 * file" across many files) legitimately gets the same/near-identical name for
 * each invocation — the naming model doing its job, not a malfunction — but
 * left flat that reads as "dozens of duplicate rows." A subagent belonging to
 * a Workflow dispatch never enters this grouping pass, regardless of name —
 * it's already represented by its one `WorkflowDispatch` row.
 */
export interface NameGroup {
    kind: "nameGroup";
    /** The shared display_name — always non-empty; grouping only fires once
     *  a name exists (display_name is null before subagent.GenerateName
     *  resolves), so ungrouped/unnamed subagents never form a NameGroup. */
    name: string;
    /** Every member's shared parent_block_id — buildDispatchChildren is
     *  always called with an already block-filtered subagent list (see
     *  buildTree()), so this is uniform across the group. Required for
     *  groupCacheKey: unlike WorkflowDispatch's backend-unique dispatchId, a
     *  Haiku-generated display_name can plausibly repeat across two
     *  unrelated agent panes (e.g. "Code Reviewer" in both), and
     *  groupIdentityCache/expandedIds are shared across the WHOLE tree —
     *  without this, two different blocks' same-named groups would stomp
     *  each other's cached identity and expand/collapse state. Reagent P1
     *  on PR #2123. */
    parentBlockId: string;
    subagents: ActiveSubagent[];
    activeCount: number;
    totalCount: number;
    status: "active" | "retired";
    lastEventAt: number;
}

export type SwarmChild = ActiveSubagent | WorkflowDispatch | NameGroup;

export function isWorkflowDispatch(child: SwarmChild): child is WorkflowDispatch {
    return "kind" in child && child.kind === "workflowDispatch";
}

export function isNameGroup(child: SwarmChild): child is NameGroup {
    return "kind" in child && child.kind === "nameGroup";
}

export interface AgentTreeNode {
    blockId: string | null;
    agentName: string;
    agentProvider: string | null;
    activitySummary: string | null;
    contextTokens: number | null;
    agentStatus: "running" | "idle";
    subagents: SwarmChild[];
}

/**
 * Build one block's tree children from its `AgentDispatch`es (already
 * block-filtered) and raw `SubAgent`s. Every Workflow-kind dispatch becomes
 * exactly one `WorkflowDispatch` row (SPEC §7 — never one row per member).
 * Every Solo-kind dispatch's one member renders as a plain `ActiveSubagent`
 * row, no wrapper (SPEC §5) — among those, two or more sharing an identical,
 * non-empty `display_name` still collapse into a `NameGroup` (grouping
 * solo DISPATCHES now, not raw subagents, but every subagent has exactly one
 * dispatch so the input set is the same). Result is sorted by most recent
 * activity, mixing loose subagents and both group kinds in one recency order.
 */
export function buildDispatchChildren(
    dispatches: AgentDispatch[],
    subagents: ActiveSubagent[]
): SwarmChild[] {
    const workflowRows: WorkflowDispatch[] = dispatches
        .filter((d) => d.kind === "workflow")
        .map((d) => {
            const namedMember = subagents.find((s) => s.dispatch_id === d.dispatch_id && s.slug);
            return {
                kind: "workflowDispatch" as const,
                dispatchId: d.dispatch_id,
                name: namedMember?.slug || d.dispatch_id,
                memberCount: d.member_count,
                membersDone: d.members_done,
                status: d.status === "completed" ? ("retired" as const) : ("active" as const),
                lastEventAt: d.last_event_at,
            };
        });

    const solo = subagents.filter((s) => s.dispatch_id.startsWith("solo:"));

    // Fallback for a failed/lagging `ListDispatches` call: `loadDispatches()`
    // swallows RPC errors and leaves `dispatchesAtom` stale (see its call
    // site), so `dispatches` here can lag or miss entries `subagents` (a
    // separate fetch) already has. Without this, any workflow-kind subagent
    // whose dispatch has no matching row in `workflowRows` would vanish from
    // the tree entirely instead of degrading to an individual row.
    const workflowDispatchIds = new Set(workflowRows.map((w) => w.dispatchId));
    const orphanedWorkflowMembers = subagents.filter(
        (s) => !s.dispatch_id.startsWith("solo:") && !workflowDispatchIds.has(s.dispatch_id)
    );

    const stillLoose: ActiveSubagent[] = [...orphanedWorkflowMembers];
    const byName = new Map<string, ActiveSubagent[]>();
    for (const s of solo) {
        if (s.display_name) {
            const members = byName.get(s.display_name) ?? [];
            members.push(s);
            byName.set(s.display_name, members);
        } else {
            stillLoose.push(s);
        }
    }

    const nameGroups: NameGroup[] = [];
    for (const [name, members] of byName) {
        if (members.length < 2) {
            stillLoose.push(...members);
            continue;
        }
        const sorted = [...members].sort((a, b) => b.last_event_at - a.last_event_at);
        const activeCount = sorted.filter((m) => m.status === "active").length;
        nameGroups.push({
            kind: "nameGroup" as const,
            name,
            // Uniform across the group — buildDispatchChildren is always
            // called with an already block-filtered list (buildTree()).
            parentBlockId: sorted[0].parent_block_id,
            subagents: sorted,
            activeCount,
            totalCount: sorted.length,
            status: activeCount > 0 ? ("active" as const) : ("retired" as const),
            lastEventAt: sorted[0]?.last_event_at ?? 0,
        });
    }

    const lastEventOf = (c: SwarmChild): number =>
        isWorkflowDispatch(c) || isNameGroup(c) ? c.lastEventAt : c.last_event_at;
    return [...stillLoose, ...workflowRows, ...nameGroups].sort((a, b) => lastEventOf(b) - lastEventOf(a));
}

/**
 * Compute the label `SubagentRow`/`SubagentDetailPane` show for a subagent,
 * in the same priority order both call sites use: `display_name` (once
 * Haiku resolves it) > `slug` > a short prefix of `agent_id`.
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
 * `buildDispatchChildren`'s `WorkflowDispatch.name` derivation above.
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
 * had — mirrors the existing pattern in `SubagentDetailPane`, which
 * already shows `agent_id.substring(0, 7)` as a separate meta chip next to
 * the name for exactly this reason.
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

/** `shallowEqualGroupContent` — type-aware, since `WorkflowDispatch` no
 *  longer carries a member list (SPEC §7 — a dispatch can have thousands of
 *  members, too many to hold in the tree atom) while `NameGroup` still does.
 *  A kind mismatch is never equal (defensive; `groupCacheKey`'s namespacing
 *  already prevents the two kinds from ever sharing a cache key). */
function shallowEqualGroupContent(a: WorkflowDispatch | NameGroup, b: WorkflowDispatch | NameGroup): boolean {
    if (isWorkflowDispatch(a) !== isWorkflowDispatch(b)) return false;
    if (isWorkflowDispatch(a) && isWorkflowDispatch(b)) {
        return (
            a.dispatchId === b.dispatchId &&
            a.name === b.name &&
            a.memberCount === b.memberCount &&
            a.membersDone === b.membersDone &&
            a.status === b.status &&
            a.lastEventAt === b.lastEventAt
        );
    }
    const na = a as NameGroup;
    const nb = b as NameGroup;
    return (
        na.name === nb.name &&
        na.activeCount === nb.activeCount &&
        na.totalCount === nb.totalCount &&
        na.status === nb.status &&
        na.lastEventAt === nb.lastEventAt &&
        na.subagents.length === nb.subagents.length &&
        na.subagents.every((m, i) => m === nb.subagents[i])
    );
}

/** Namespaced cache key so a `WorkflowDispatch`'s `dispatchId` and a
 *  `NameGroup`'s `name` can never collide in the shared identity cache.
 *  `NameGroup` additionally scopes by `parentBlockId`: a `dispatchId` is
 *  backend-unique so a bare name would never collide there, but a
 *  Haiku-generated `display_name` (e.g. "Code Reviewer") can plausibly
 *  repeat across two unrelated agent panes, and `groupIdentityCache`/
 *  `expandedIds` are shared across the WHOLE tree (every block) — without
 *  the block scope, two blocks' same-named groups would stomp each other's
 *  cached identity and expand/collapse state. Reagent P1 on PR #2123. */
export function groupCacheKey(child: WorkflowDispatch | NameGroup): string {
    return isWorkflowDispatch(child)
        ? `wf:${child.dispatchId}`
        : `name:${child.parentBlockId}:${child.name}`;
}

/**
 * Stabilize `WorkflowDispatch`/`NameGroup` wrapper identity across
 * `buildTree()` calls, mirroring `mergeSubagentsPreservingIdentity` one
 * level up. `buildDispatchChildren` is a pure function — it unconditionally
 * builds brand-new group objects per call, even when nothing about a given
 * group actually changed. Left alone, that fresh wrapper still remounts
 * `WorkflowDispatchRow`/`NameGroupRow` — and everything nested inside an
 * expanded one — on every unrelated tree recompute, which for a large
 * workflow dispatch defeats the very remount fix `expandedIdsAtom`/
 * `getDispatchDetail` were meant to provide.
 *
 * `cache` is a `Map<groupCacheKey, group>` the caller keeps around (one per
 * `SwarmViewModel`), shared and called once per BLOCK within one
 * `buildTree()` pass — this function only reuses-or-replaces into `cache`,
 * it never prunes it (pruning here, scoped to one block's children, would
 * evict entries a DIFFERENT block's call just wrote). Callers doing a full
 * multi-block tree rebuild should prune the cache once afterward against
 * every group key actually produced across the whole tree — see
 * `pruneGroupIdentityCache`.
 */
export function stabilizeGroupIdentity(
    cache: Map<string, WorkflowDispatch | NameGroup>,
    children: SwarmChild[]
): SwarmChild[] {
    return children.map((child) => {
        if (!isWorkflowDispatch(child) && !isNameGroup(child)) return child;
        const key = groupCacheKey(child);
        const old = cache.get(key);
        const stable = old && shallowEqualGroupContent(old, child) ? old : child;
        cache.set(key, stable);
        return stable;
    });
}

/** Drop cache entries for groups no longer present anywhere in the tree —
 *  call once after a full `buildTree()` pass, not per-block (see
 *  `stabilizeGroupIdentity`), so it can't grow unbounded across sessions.
 *  `liveGroupKeys` must use the same `wf:<id>` / `name:<name>` namespacing
 *  as `groupCacheKey` — see `SwarmViewModel.buildTree()`. */
export function pruneGroupIdentityCache(cache: Map<string, WorkflowDispatch | NameGroup>, liveGroupKeys: Set<string>): void {
    for (const key of [...cache.keys()]) {
        if (!liveGroupKeys.has(key)) cache.delete(key);
    }
}

// ── Subagent detail (inline-expand event log) ───────────────────────────

export interface SubagentDetail {
    eventsAtom: Accessor<SubagentEvent[]>;
    infoAtom: Accessor<ActiveSubagent | null>;
    statusAtom: Accessor<"active" | "completed" | "abandoned" | "loading">;
    dispose: () => void;
}

/**
 * Plain-function counterpart of the retired SubagentViewModel — fetches one
 * subagent's event history + info once and keeps them live via
 * subagent:activity/subagent:completed. Deliberately NOT tied to a
 * component's render lifecycle (no `onCleanup`): `SwarmViewModel` caches
 * one of these per expanded subagent (`getSubagentDetail`) so it survives
 * `<For>` remounts `tree()` can still cause for unrelated reasons, instead
 * of refetching/resubscribing on every incidental re-render while a row is
 * open. See `mergeSubagentsPreservingIdentity` and `stabilizeGroupIdentity`
 * above for how row/wrapper object identity itself is kept stable in the
 * first place.
 */
export function createSubagentDetail(subagentId: string): SubagentDetail {
    const [events, setEvents] = createSignal<SubagentEvent[]>([]);
    const [info, setInfo] = createSignal<ActiveSubagent | null>(null);
    const [status, setStatus] = createSignal<"active" | "completed" | "abandoned" | "loading">("loading");

    const unsubs: (() => void)[] = [];

    const unsubActivity = waveEventSubscribe({
        eventType: "subagent:activity",
        handler: (event: WaveEvent) => {
            const data = event?.data as any;
            if (data?.agentId !== subagentId) return;
            const newEvents = (data?.events as SubagentEvent[]) ?? [];
            if (newEvents.length > 0) setEvents((prev) => [...prev, ...newEvents]);
            if (data?.totalEvents != null) {
                setInfo((prev) => (prev ? { ...prev, event_count: data.totalEvents } : prev));
            }
        },
    });
    if (unsubActivity) unsubs.push(unsubActivity);

    const unsubCompleted = waveEventSubscribe({
        eventType: "subagent:completed",
        handler: (event: WaveEvent) => {
            const data = event?.data as any;
            if (data?.agentId !== subagentId) return;
            setStatus("completed");
            setInfo((prev) => (prev ? { ...prev, status: "completed" } : prev));
        },
    });
    if (unsubCompleted) unsubs.push(unsubCompleted);

    void (async () => {
        try {
            const result = await callBackendService("subagent", "GetHistory", [subagentId, 500]);
            setEvents((result as SubagentEvent[]) ?? []);
            setStatus("active");
        } catch {
            setStatus("active");
        }
        // Targeted single-agent lookup (not ListActive's full scan) — this
        // subagent may have spawned before this pane opened, so its
        // subagent:spawned event already fired and won't refire.
        try {
            const result = await callBackendService("subagent", "GetInfo", [subagentId]);
            if (result) {
                const match = result as ActiveSubagent;
                setInfo(match);
                setStatus(match.status);
            }
        } catch {
            // ignore
        }
    })();

    return {
        eventsAtom: events,
        infoAtom: info,
        statusAtom: status,
        dispose: () => {
            for (const unsub of unsubs) unsub();
        },
    };
}

// ── Dispatch concatenated activity feed (SPEC §7) ───────────────────────

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

/**
 * Expanding a `WorkflowDispatchRow` shows this instead of nested member rows
 * (SPEC §7): every member's new events, merged into one chronological,
 * member-tagged stream, fed by the backend's coalesced `dispatch:activity`
 * broadcast (`subagent_watcher.rs`'s `flush_pending_dispatch_activity`).
 *
 * Deliberately LIVE-ONLY — no historical backfill fetch when a dispatch is
 * expanded. SPEC §9.4 leaves the backfill/pagination question explicitly
 * open: a large dispatch can have thousands of prior events across hundreds
 * of members, and eagerly fetching + merging that on every expand would
 * reintroduce the exact request/render volume problem this redesign exists
 * to fix. Until that's designed, the feed only shows what happens from the
 * moment of expand onward.
 */
export function createDispatchDetail(dispatchId: string): DispatchDetail {
    const [entries, setEntries] = createSignal<DispatchActivityEntry[]>([]);

    const unsub = waveEventSubscribe({
        eventType: "dispatch:activity",
        handler: (event: WaveEvent) => {
            const data = event?.data as any;
            if (data?.dispatchId !== dispatchId) return;
            const members = (data?.members as { agentId: string; events: SubagentEvent[] }[]) ?? [];
            const incoming: DispatchActivityEntry[] = members.flatMap((m) =>
                (m.events ?? []).map((event) => ({ agentId: m.agentId, event }))
            );
            if (incoming.length === 0) return;
            setEntries((prev) => {
                const merged = [...prev, ...incoming].sort((a, b) => a.event.timestamp - b.event.timestamp);
                return merged.length > MAX_DISPATCH_FEED_ENTRIES
                    ? merged.slice(merged.length - MAX_DISPATCH_FEED_ENTRIES)
                    : merged;
            });
        },
    });

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

    // Rows the user has expanded — keyed by dispatchId (WorkflowDispatchRow)
    // or agent_id (SubagentRow). Lives here, not as row-local component state:
    // `tree()` recomputes on every trackedBlockIds/subagents/agentStatuses
    // change (agentStatuses updates on every controllerstatus tick — very
    // frequent during an active turn) and rebuilds fresh WorkflowDispatch/
    // AgentTreeNode wrapper objects every time regardless of whether that
    // row's own data changed, so `<For>`'s reference-diffing remounts row
    // components far more often than "the user actually changed something."
    // Local expand state would silently collapse on the very next unrelated
    // status tick; keying by a stable string id here survives that churn.
    private _expandedIds = createSignal<Set<string>>(new Set());
    expandedIdsAtom: Accessor<Set<string>> = this._expandedIds[0];
    private setExpandedIds: Setter<Set<string>> = this._expandedIds[1];

    // Top-level agent rows collapse with the OPPOSITE default from the group/
    // subagent rows above: agents default EXPANDED, so this tracks the
    // exception set (collapsed blockIds) rather than reusing expandedIds
    // (whose absent-means-collapsed semantics would flip every agent shut on
    // mount). Same ViewModel-residency rationale as _expandedIds: tree()
    // rebuilds wrapper objects on every status tick, so row-local state would
    // silently reset on unrelated refreshes.
    private _collapsedAgentIds = createSignal<Set<string>>(new Set());
    collapsedAgentIdsAtom: Accessor<Set<string>> = this._collapsedAgentIds[0];
    private setCollapsedAgentIds: Setter<Set<string>> = this._collapsedAgentIds[1];

    // One SubagentDetail per currently-expanded subagent, created lazily on
    // first expand. Same rationale as expandedIds above: if this fetch+
    // subscribe lifecycle lived inside the row component instead, every
    // incidental remount (see above) would refetch GetHistory/GetInfo and
    // resubscribe from scratch — potentially several times a second while a
    // row is open during an active turn.
    private detailCache = new Map<string, SubagentDetail>();

    // One DispatchDetail (concatenated activity feed, SPEC §7) per
    // currently-expanded WorkflowDispatch row — same lazy-create/dispose-on-
    // collapse rationale as detailCache above.
    private dispatchDetailCache = new Map<string, DispatchDetail>();

    // Persisted across buildTree() calls so stabilizeGroupIdentity can reuse
    // the same WorkflowDispatch/NameGroup wrapper object when a group's own
    // content is unchanged — without this, expandedIds/detailCache above
    // still don't help a subagent NESTED inside a group, since <For> remounts
    // the whole WorkflowDispatchRow/NameGroupRow subtree whenever the
    // group's own wrapper reference changes, which is every buildTree() call
    // otherwise. Keyed by groupCacheKey's namespaced "wf:<id>" / "name:<name>"
    // strings so the two group kinds' key spaces never collide.
    private groupIdentityCache = new Map<string, WorkflowDispatch | NameGroup>();

    private unsubs: (() => void)[] = [];
    // Per-block controllerstatus unsubs — cleaned up when block list refreshes
    private blockUnsubs: (() => void)[] = [];

    // Backend broadcasts one subagent:spawned/subagent:completed event per
    // subagent file (see subagent_watcher.rs's process_jsonl_change) — a
    // backfill scan on pane reopen can fire dozens of these in a burst.
    // Debounce the resulting loadSubagents() RPC here instead of batching the
    // broadcasts themselves, since useSubagentEvents.ts (a different consumer
    // of the same events) needs one event per subagent to populate its
    // per-agent document nodes.
    private loadSubagentsDebounceTimer: ReturnType<typeof setTimeout> | undefined;
    private static readonly LOAD_SUBAGENTS_DEBOUNCE_MS = 150;

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

        // dispatch:updated fires for both Solo and Workflow kinds (SPEC §5) —
        // a member count/status change on either warrants the same reload
        // used for subagent spawn/completion (they're closely correlated:
        // a workflow member spawning/completing IS a dispatch update).
        const unsubDispatchUpdated = waveEventSubscribe({
            eventType: "dispatch:updated",
            handler: () => this.scheduleLoadSubagents(),
        });
        if (unsubDispatchUpdated) this.unsubs.push(unsubDispatchUpdated);

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
            await Promise.all([this.loadTrackedBlocks(), this.loadSubagents(), this.loadDispatches()]);
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
            void this.loadSubagents();
            void this.loadDispatches();
        }, SwarmViewModel.LOAD_SUBAGENTS_DEBOUNCE_MS);
    };

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

    /** Top-level AgentRow collapse — see _collapsedAgentIds for the
     *  inverted (default-expanded) semantics vs isExpanded/toggleExpanded. */
    isAgentCollapsed(blockId: string): boolean {
        return this.collapsedAgentIdsAtom().has(blockId);
    }

    toggleAgentCollapsed(blockId: string): void {
        this.setCollapsedAgentIds((prev) => {
            const next = new Set(prev);
            if (next.has(blockId)) next.delete(blockId);
            else next.add(blockId);
            return next;
        });
    }

    /** SubagentRow's toggle — also tears down the cached SubagentDetail
     *  (and its WS subscriptions) on collapse, so a subagent a user opened
     *  once and moved on from doesn't keep a live subscription forever. */
    toggleSubagentExpanded(agentId: string): void {
        const wasExpanded = this.isExpanded(agentId);
        this.toggleExpanded(agentId);
        if (wasExpanded) {
            this.detailCache.get(agentId)?.dispose();
            this.detailCache.delete(agentId);
        }
    }

    getSubagentDetail(agentId: string): SubagentDetail {
        let detail = this.detailCache.get(agentId);
        if (!detail) {
            detail = createSubagentDetail(agentId);
            this.detailCache.set(agentId, detail);
        }
        return detail;
    }

    /** WorkflowDispatchRow's toggle — mirrors toggleSubagentExpanded, tearing
     *  down the cached DispatchDetail (and its dispatch:activity
     *  subscription) on collapse. */
    toggleDispatchExpanded(dispatchId: string): void {
        const wasExpanded = this.isExpanded(dispatchId);
        this.toggleExpanded(dispatchId);
        if (wasExpanded) {
            this.dispatchDetailCache.get(dispatchId)?.dispose();
            this.dispatchDetailCache.delete(dispatchId);
        }
    }

    getDispatchDetail(dispatchId: string): DispatchDetail {
        let detail = this.dispatchDetailCache.get(dispatchId);
        if (!detail) {
            detail = createDispatchDetail(dispatchId);
            this.dispatchDetailCache.set(dispatchId, detail);
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
        const statuses = this.agentStatusesAtom();

        // Include parent block IDs from subagents as fallback for agent panes
        // that registered subagents before their own registration propagated.
        const parentIds = subagents.map((s) => s.parent_block_id).filter(Boolean);
        const allBlockIds = [...new Set([...blockIds, ...parentIds])];

        // Collected from the groups actually produced below, not derived
        // from the raw subagent list — a NameGroup only exists once 2+
        // subagents share a name (see buildDispatchChildren), so a
        // subagent-list-level heuristic ("any subagent with this name is
        // live") would wrongly keep singleton entries' keys alive forever.
        // Harmless either way (stabilizeGroupIdentity only ever cache.set()s
        // keys for groups that actually formed), but this is the precise set.
        const liveGroupKeys = new Set<string>();
        const nodes = allBlockIds.map((blockId) => {
            const blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
            const block = blockAtom();
            const agentName =
                (block?.meta?.["agentName"] as string | undefined)?.trim() ||
                "Agent";
            const agentProvider =
                (block?.meta?.["agentProvider"] as string | undefined)?.trim() || null;
            const activitySummary = readActivitySummary(block?.meta)?.trim() || null;
            const rawCtx = block?.meta?.["term:ctx-tokens"];
            const contextTokens = typeof rawCtx === "number" ? rawCtx : null;
            const agentStatus = statuses.get(blockId) ?? "idle";
            const rawChildren = buildDispatchChildren(
                dispatches.filter((d) => d.parent_block_id === blockId),
                subagents.filter((s) => s.parent_block_id === blockId)
            );
            for (const c of rawChildren) {
                if (isWorkflowDispatch(c) || isNameGroup(c)) liveGroupKeys.add(groupCacheKey(c));
            }
            const children = stabilizeGroupIdentity(this.groupIdentityCache, rawChildren);
            return { blockId, agentName, agentProvider, activitySummary, contextTokens, agentStatus, subagents: children };
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
        for (const detail of this.detailCache.values()) detail.dispose();
        this.detailCache.clear();
        for (const detail of this.dispatchDetailCache.values()) detail.dispose();
        this.dispatchDetailCache.clear();
        this.groupIdentityCache.clear();
    }
}
