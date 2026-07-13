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

export interface ActiveSubagent {
    agent_id: string;
    slug: string;
    parent_agent: string;
    parent_block_id: string;
    session_id: string;
    status: "active" | "completed";
    /** Unix ms when this subagent was first observed — set once, immutable.
     *  Distinct from `last_event_at`, which advances on every journal read. */
    spawned_at: number;
    last_event_at: number;
    event_count: number;
    model: string | null;
    // Already on the wire (SubagentInfo.workflow_id, Rust) — was previously
    // typed away here. Some("wf_<id>") for a Task/Workflow-tool run that
    // spawned multiple subagents together; null for a standalone subagent.
    workflow_id: string | null;
    // Concise Haiku-generated name (SubagentInfo.display_name, Rust). Null
    // until a client expands this subagent's row for the first time — see
    // `subagent.GenerateName` / the `subagent:named` event below.
    display_name: string | null;
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
 * A group of subagents spawned together by one Task/Workflow-tool run
 * (shared `workflow_id`). Collapsed into one row in the tree instead of one
 * row per member — a single workflow run can spawn dozens of subagents at
 * once (observed live: 45), which read as a "flood" when listed flat. See
 * docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md Finding 4.
 */
export interface WorkflowGroup {
    kind: "workflowGroup";
    workflowId: string;
    /** Derived client-side from the first member with a non-empty slug —
     *  the backend has no separate workflow-name concept (see report). */
    name: string;
    subagents: ActiveSubagent[];
    activeCount: number;
    totalCount: number;
    /** "active" if any member is still active; "retired" once every member
     *  has completed. */
    status: "active" | "retired";
    lastEventAt: number;
}

/**
 * A group of LOOSE subagents (no `workflow_id` — not spawned together by one
 * Task/Workflow-tool run) that independently earned the same Haiku-generated
 * `display_name`. A user repeatedly spawning similar tasks (e.g. "review this
 * file" across many files) legitimately gets the same/near-identical name for
 * each invocation — the naming model doing its job, not a malfunction — but
 * left flat that reads as "dozens of duplicate rows." Workflow grouping takes
 * priority: a subagent already collapsed into a `WorkflowGroup` never also
 * enters this grouping pass, regardless of name. See
 * docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md — this extends
 * that report's chosen "group, not truncate" approach to same-name loose
 * subagents, which the original workflow-only grouping didn't cover.
 */
export interface NameGroup {
    kind: "nameGroup";
    /** The shared display_name — always non-empty; grouping only fires once
     *  a name exists (display_name is null before subagent.GenerateName
     *  resolves), so ungrouped/unnamed subagents never form a NameGroup. */
    name: string;
    /** Every member's shared parent_block_id — groupSubagentsByWorkflow is
     *  always called with an already block-filtered subagent list (see
     *  buildTree()), so this is uniform across the group. Required for
     *  groupCacheKey: unlike WorkflowGroup's backend-unique workflowId, a
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

export type SwarmChild = ActiveSubagent | WorkflowGroup | NameGroup;

export function isWorkflowGroup(child: SwarmChild): child is WorkflowGroup {
    return "kind" in child && child.kind === "workflowGroup";
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
 * Group `subagents` (already filtered to one parent block) by `workflow_id`,
 * then, for the loose remainder, by shared `display_name`. Subagents sharing
 * a `workflow_id` collapse into a `WorkflowGroup`; among what's left, two or
 * more subagents sharing an identical, non-empty `display_name` collapse
 * into a `NameGroup`. A single subagent with a unique name (or no name yet)
 * stays a loose, ungrouped row — group chrome for something that isn't
 * actually a dupe would be noise of its own. Result is sorted by most recent
 * activity, mixing loose subagents and both group kinds in one recency order.
 */
export function groupSubagentsByWorkflow(subagents: ActiveSubagent[]): SwarmChild[] {
    const loose: ActiveSubagent[] = [];
    const byWorkflow = new Map<string, ActiveSubagent[]>();
    for (const s of subagents) {
        if (s.workflow_id) {
            const members = byWorkflow.get(s.workflow_id) ?? [];
            members.push(s);
            byWorkflow.set(s.workflow_id, members);
        } else {
            loose.push(s);
        }
    }

    const groups: WorkflowGroup[] = [...byWorkflow.entries()].map(([workflowId, members]) => {
        const sorted = [...members].sort((a, b) => b.last_event_at - a.last_event_at);
        const activeCount = sorted.filter((m) => m.status === "active").length;
        return {
            kind: "workflowGroup" as const,
            workflowId,
            name: sorted.find((m) => m.slug)?.slug || workflowId,
            subagents: sorted,
            activeCount,
            totalCount: sorted.length,
            status: activeCount > 0 ? "active" as const : "retired" as const,
            lastEventAt: sorted[0]?.last_event_at ?? 0,
        };
    });

    const stillLoose: ActiveSubagent[] = [];
    const byName = new Map<string, ActiveSubagent[]>();
    for (const s of loose) {
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
            // Uniform across the group — groupSubagentsByWorkflow is always
            // called with an already block-filtered list (buildTree()).
            parentBlockId: sorted[0].parent_block_id,
            subagents: sorted,
            activeCount,
            totalCount: sorted.length,
            status: activeCount > 0 ? "active" as const : "retired" as const,
            lastEventAt: sorted[0]?.last_event_at ?? 0,
        });
    }

    const lastEventOf = (c: SwarmChild): number =>
        isWorkflowGroup(c) || isNameGroup(c) ? c.lastEventAt : c.last_event_at;
    return [...stillLoose, ...groups, ...nameGroups].sort((a, b) => lastEventOf(b) - lastEventOf(a));
}

function shallowEqualSubagent(a: ActiveSubagent, b: ActiveSubagent): boolean {
    return (
        a.slug === b.slug &&
        a.status === b.status &&
        a.last_event_at === b.last_event_at &&
        a.event_count === b.event_count &&
        a.model === b.model &&
        a.workflow_id === b.workflow_id &&
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

/** Common fields shared by both group kinds — `shallowEqualGroupContent`
 *  compares only these, so it works for a `WorkflowGroup` or `NameGroup`
 *  pair without needing to know which kind it's looking at. */
function shallowEqualGroupContent(a: WorkflowGroup | NameGroup, b: WorkflowGroup | NameGroup): boolean {
    return (
        a.name === b.name &&
        a.activeCount === b.activeCount &&
        a.totalCount === b.totalCount &&
        a.status === b.status &&
        a.lastEventAt === b.lastEventAt &&
        a.subagents.length === b.subagents.length &&
        a.subagents.every((m, i) => m === b.subagents[i])
    );
}

/** Namespaced cache key so a `WorkflowGroup`'s `workflowId` and a
 *  `NameGroup`'s `name` can never collide in the shared identity cache.
 *  `NameGroup` additionally scopes by `parentBlockId`: a `workflowId` is
 *  backend-unique so a bare name would never collide there, but a
 *  Haiku-generated `display_name` (e.g. "Code Reviewer") can plausibly
 *  repeat across two unrelated agent panes, and `groupIdentityCache`/
 *  `expandedIds` are shared across the WHOLE tree (every block) — without
 *  the block scope, two blocks' same-named groups would stomp each other's
 *  cached identity and expand/collapse state. Reagent P1 on PR #2123. */
export function groupCacheKey(child: WorkflowGroup | NameGroup): string {
    return isWorkflowGroup(child)
        ? `wf:${child.workflowId}`
        : `name:${child.parentBlockId}:${child.name}`;
}

/**
 * Stabilize `WorkflowGroup`/`NameGroup` wrapper identity across `buildTree()`
 * calls, mirroring `mergeSubagentsPreservingIdentity` one level up.
 * `groupSubagentsByWorkflow` is a pure function — it unconditionally builds
 * brand-new group objects per call, even when every member (already
 * reference-stable thanks to `mergeSubagentsPreservingIdentity`) is
 * unchanged. Left alone, that fresh wrapper still remounts `WorkflowGroupRow`/
 * `NameGroupRow` — and everything nested inside an expanded one
 * (`SubagentRow`, `SubagentDetailPane`, `SubagentDetailEvent`) — on every
 * unrelated tree recompute, which for a workflow group (the highest-volume
 * case: a single run can spawn dozens of subagents) defeats the very
 * remount fix `expandedIdsAtom`/`getSubagentDetail` were meant to provide.
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
    cache: Map<string, WorkflowGroup | NameGroup>,
    children: SwarmChild[]
): SwarmChild[] {
    return children.map((child) => {
        if (!isWorkflowGroup(child) && !isNameGroup(child)) return child;
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
export function pruneGroupIdentityCache(cache: Map<string, WorkflowGroup | NameGroup>, liveGroupKeys: Set<string>): void {
    for (const key of [...cache.keys()]) {
        if (!liveGroupKeys.has(key)) cache.delete(key);
    }
}

// ── Subagent detail (inline-expand event log) ───────────────────────────

export interface SubagentDetail {
    eventsAtom: Accessor<SubagentEvent[]>;
    infoAtom: Accessor<ActiveSubagent | null>;
    statusAtom: Accessor<"active" | "completed" | "loading">;
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
    const [status, setStatus] = createSignal<"active" | "completed" | "loading">("loading");

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

    // Rows the user has expanded — keyed by workflowId (WorkflowGroupRow) or
    // agent_id (SubagentRow). Lives here, not as row-local component state:
    // `tree()` recomputes on every trackedBlockIds/subagents/agentStatuses
    // change (agentStatuses updates on every controllerstatus tick — very
    // frequent during an active turn) and rebuilds fresh WorkflowGroup/
    // AgentTreeNode wrapper objects every time regardless of whether that
    // row's own data changed, so `<For>`'s reference-diffing remounts row
    // components far more often than "the user actually changed something."
    // Local expand state would silently collapse on the very next unrelated
    // status tick; keying by a stable string id here survives that churn.
    private _expandedIds = createSignal<Set<string>>(new Set());
    expandedIdsAtom: Accessor<Set<string>> = this._expandedIds[0];
    private setExpandedIds: Setter<Set<string>> = this._expandedIds[1];

    // One SubagentDetail per currently-expanded subagent, created lazily on
    // first expand. Same rationale as expandedIds above: if this fetch+
    // subscribe lifecycle lived inside the row component instead, every
    // incidental remount (see above) would refetch GetHistory/GetInfo and
    // resubscribe from scratch — potentially several times a second while a
    // row is open during an active turn.
    private detailCache = new Map<string, SubagentDetail>();

    // Persisted across buildTree() calls so stabilizeGroupIdentity can reuse
    // the same WorkflowGroup/NameGroup wrapper object when a group's own
    // content is unchanged — without this, expandedIds/detailCache above
    // still don't help a subagent NESTED inside a group, since <For> remounts
    // the whole WorkflowGroupRow/NameGroupRow subtree (SubagentRow,
    // SubagentDetailPane, SubagentDetailEvent — including the latter's own
    // local `expanded` signal for a tool_use/tool_result toggle) whenever the
    // group's own wrapper reference changes, which is every buildTree() call
    // otherwise. Keyed by groupCacheKey's namespaced "wf:<id>" / "name:<name>"
    // strings so the two group kinds' key spaces never collide.
    private groupIdentityCache = new Map<string, WorkflowGroup | NameGroup>();

    private unsubs: (() => void)[] = [];
    // Per-block controllerstatus unsubs — cleaned up when block list refreshes
    private blockUnsubs: (() => void)[] = [];

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        void this.loadAll();

        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: () => void this.loadSubagents(),
        });
        if (unsubSpawned) this.unsubs.push(unsubSpawned);

        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: () => void this.loadSubagents(),
        });
        if (unsubCompleted) this.unsubs.push(unsubCompleted);

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
            await Promise.all([this.loadTrackedBlocks(), this.loadSubagents()]);
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
        const statuses = this.agentStatusesAtom();

        // Include parent block IDs from subagents as fallback for agent panes
        // that registered subagents before their own registration propagated.
        const parentIds = subagents.map((s) => s.parent_block_id).filter(Boolean);
        const allBlockIds = [...new Set([...blockIds, ...parentIds])];

        // Collected from the groups actually produced below, not derived
        // from the raw subagent list — a NameGroup only exists once 2+
        // subagents share a name (see groupSubagentsByWorkflow), so a
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
            const rawChildren = groupSubagentsByWorkflow(subagents.filter((s) => s.parent_block_id === blockId));
            for (const c of rawChildren) {
                if (isWorkflowGroup(c) || isNameGroup(c)) liveGroupKeys.add(groupCacheKey(c));
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
        for (const detail of this.detailCache.values()) detail.dispose();
        this.detailCache.clear();
        this.groupIdentityCache.clear();
    }
}
