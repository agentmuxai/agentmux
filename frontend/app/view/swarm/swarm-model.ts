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
    last_event_at: number;
    event_count: number;
    model: string | null;
    // Already on the wire (SubagentInfo.workflow_id, Rust) — was previously
    // typed away here. Some("wf_<id>") for a Task/Workflow-tool run that
    // spawned multiple subagents together; null for a standalone subagent.
    workflow_id: string | null;
}

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

export type SwarmChild = ActiveSubagent | WorkflowGroup;

export function isWorkflowGroup(child: SwarmChild): child is WorkflowGroup {
    return "kind" in child && child.kind === "workflowGroup";
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
 * Group `subagents` (already filtered to one parent block) by `workflow_id`.
 * Subagents with no `workflow_id` pass through unchanged; subagents sharing
 * one collapse into a single `WorkflowGroup`. Result is sorted by most
 * recent activity, mixing loose subagents and groups in one recency order.
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

    const lastEventOf = (c: SwarmChild): number => (isWorkflowGroup(c) ? c.lastEventAt : c.last_event_at);
    return [...loose, ...groups].sort((a, b) => lastEventOf(b) - lastEventOf(a));
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

    // Workflow groups the user has expanded, keyed by workflowId. Lives
    // here, not as WorkflowGroupRow-local component state: `buildTree()`
    // calls `groupSubagentsByWorkflow`, which builds brand-new WorkflowGroup
    // wrapper objects on every recompute — including when an UNRELATED block
    // changes (any subagent spawn/complete event, any `term:ctx-tokens` or
    // activity-summary meta update touches `tree()`'s dependencies). Since
    // `<For>` diffs list items by reference, a fresh wrapper object remounts
    // WorkflowGroupRow and silently resets a local `expanded` signal on
    // essentially every tree refresh — precisely while watching an active
    // workflow's progress, the case this feature targets. Keying expand
    // state by a stable string id here survives that churn.
    private _expandedIds = createSignal<Set<string>>(new Set());
    expandedIdsAtom: Accessor<Set<string>> = this._expandedIds[0];
    private setExpandedIds: Setter<Set<string>> = this._expandedIds[1];

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
            this.setSubagents(list);
        } catch {
            // silently ignore
        }
    };

    isExpanded(id: string): boolean {
        return this.expandedIdsAtom().has(id);
    }

    toggleExpanded(id: string): void {
        this.setExpandedIds((prev) => {
            const next = new Set(prev);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
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

        return allBlockIds.map((blockId) => {
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
            const children = groupSubagentsByWorkflow(
                subagents.filter((s) => s.parent_block_id === blockId)
            );
            return { blockId, agentName, agentProvider, activitySummary, contextTokens, agentStatus, subagents: children };
        });
    }

    dispose(): void {
        for (const unsub of [...this.unsubs, ...this.blockUnsubs]) unsub();
        this.unsubs = [];
        this.blockUnsubs = [];
    }
}
