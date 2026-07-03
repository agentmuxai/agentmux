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
}

export interface AgentTreeNode {
    blockId: string | null;
    agentName: string;
    agentProvider: string | null;
    activitySummary: string | null;
    contextTokens: number | null;
    agentStatus: "running" | "idle";
    subagents: ActiveSubagent[];
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

        // Block:activity meta changes (term:activity) — force re-read of block meta.
        // The block atom in WOS updates reactively, so the memo in the view
        // already reacts; no explicit handler needed here beyond the WOS atom.
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
                    const status = rts?.shellprocstatus === "running" ? "running" : "idle";
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
                    const proc = (ev as any)?.data?.shellprocstatus as string | undefined;
                    const next: "running" | "idle" = proc === "running" ? "running" : "idle";
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
            const children = subagents
                .filter((s) => s.parent_block_id === blockId)
                .sort((a, b) => b.last_event_at - a.last_event_at);
            return { blockId, agentName, agentProvider, activitySummary, contextTokens, agentStatus, subagents: children };
        });
    }

    dispose(): void {
        for (const unsub of [...this.unsubs, ...this.blockUnsubs]) unsub();
        this.unsubs = [];
        this.blockUnsubs = [];
    }
}
