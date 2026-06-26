// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel, AgentTreeNode, ActiveSubagent } from "./swarm-model";
import { ProviderLogo } from "@/app/element/ProviderLogo";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS, workspace, setActiveTab, atoms, getApi } from "@/app/store/global";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { getBlockTurnPhase } from "@/app/store/agentActivity";
import { WorkspaceService } from "@/app/store/services";
import "./swarm-view.scss";

// Navigate to the pane for a given block ID, switching tabs and windows as needed.
// Tries the current window first (fast path), then searches all other workspaces
// and brings the containing window to focus.
async function focusBlock(blockId: string): Promise<void> {
    // Fast path: search the current window's workspace.
    const ws = workspace();
    if (ws) {
        const allTabIds = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
        for (const tabId of allTabIds) {
            const layoutModel = getLayoutModelForTabById(tabId);
            const node = layoutModel?.getNodeByBlockId(blockId);
            if (node?.id != null) {
                await setActiveTab(tabId);
                layoutModel.focusNode(node.id);
                return;
            }
        }
    }
    // Slow path: agent is in a different window — query all workspaces.
    const allWorkspaces = await RpcApi.WorkspaceListCommand(TabRpcClient);
    for (const wsInfo of allWorkspaces) {
        if (wsInfo.workspacedata.oid === ws?.oid) continue;
        const wsData = wsInfo.workspacedata;
        const allTabIds = [...(wsData.pinnedtabids ?? []), ...(wsData.tabids ?? [])];
        for (const tabId of allTabIds) {
            const layoutModel = getLayoutModelForTabById(tabId);
            const node = layoutModel?.getNodeByBlockId(blockId);
            if (node?.id != null) {
                await WorkspaceService.SetActiveTab(wsData.oid, tabId);
                const instances = await getApi().listWindowInstances();
                const instance = instances.find((i) => i.windowId === wsInfo.windowid);
                if (instance?.label) {
                    await getApi().focusWindow(instance.label);
                }
                layoutModel.focusNode(node.id);
                return;
            }
        }
    }
}

export function SwarmView(props: ViewComponentProps<SwarmViewModel>): JSX.Element {
    const model = props.model;
    const block = WOS.getWaveObjectAtom<Block>(`block:${model.blockId}`);

    const zoomFactor = createMemo(() => {
        const z = block()?.meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });
    const setZoom = (next: number): void => {
        const clamped = Math.max(0.5, Math.min(2.0, Math.round(next * 100) / 100));
        void RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", model.blockId),
            meta: { "term:zoom": clamped === 1.0 ? null : clamped },
        });
    };

    let rootRef: HTMLDivElement | undefined;

    onMount(() => {
        if (!rootRef) return;
        const el = rootRef;
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            setZoom(zoomFactor() + (ev.deltaY > 0 ? -STEP : STEP));
        };
        el.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => el.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
    });

    onMount(() => {
        if (!rootRef) return;
        const el = rootRef;
        const handleKey = (ev: KeyboardEvent) => {
            if (!ev.ctrlKey || ev.altKey || ev.metaKey) return;
            if (!(ev.target instanceof Node) || !el.contains(ev.target)) return;
            const STEP = 0.1;
            if (ev.key === "+" || ev.key === "=") {
                ev.preventDefault();
                setZoom(zoomFactor() + STEP);
            } else if (ev.key === "-" || ev.key === "_") {
                ev.preventDefault();
                setZoom(zoomFactor() - STEP);
            } else if (ev.key === "0") {
                ev.preventDefault();
                setZoom(1.0);
            }
        };
        document.addEventListener("keydown", handleKey, { capture: true });
        onCleanup(() => document.removeEventListener("keydown", handleKey, { capture: true }));
    });

    const tree = createMemo(() => model.buildTree());

    // Derive the currently-focused block ID from the active tab's layout model.
    // focusedNode is already a reactive memo on LayoutModel, so this updates
    // automatically when the user clicks any tile or switches tabs.
    const focusedBlockId = createMemo<string | null>(() => {
        const tabId = atoms.activeTabId();
        if (!tabId) return null;
        const layoutModel = getLayoutModelForTabById(tabId);
        if (!layoutModel) return null;
        return layoutModel.focusedNode()?.data?.blockId ?? null;
    });

    return (
        <div ref={rootRef} class="swarm-view" style={{ zoom: zoomFactor() }} tabIndex={-1}>
            <Show
                when={!model.loadingAtom()}
                fallback={<div class="swarm-loading">Loading…</div>}
            >
                <Show
                    when={tree().length > 0}
                    fallback={
                        <div class="swarm-empty-root">
                            <span class="swarm-empty-icon">⬡</span>
                            <div class="swarm-empty-title">No active agent panes</div>
                            <div class="swarm-empty-desc">
                                Start an agent session to see it here.
                            </div>
                        </div>
                    }
                >
                    <div class="swarm-tree">
                        <For each={tree()}>
                            {(node) => <AgentRow node={node} focusedBlockId={focusedBlockId} />}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

// ── Status derived from TurnPhase ────────────────────────────────────────

type AgentDisplayStatus = "working" | "tools" | "stopping" | "idle" | "error" | "disconnected";

function phaseToDisplayStatus(blockId: string, _fallback: "running" | "idle"): AgentDisplayStatus {
    const phaseAccessor = getBlockTurnPhase(blockId);
    // shellprocstatus "running" only means the process is alive, not that it's
    // doing LLM work. Without a registered TurnPhase, default to idle.
    if (!phaseAccessor) return "idle";
    const phase = phaseAccessor();
    switch (phase.kind) {
        case "Submitting":    return "working";
        case "Streaming":     return phase.toolsActive > 0 ? "tools" : "working";
        case "Interrupting":  return "stopping";
        case "Done":          return phase.outcome === "errored" ? "error" : "idle";
        case "Disconnected":  return "disconnected";
        default:              return "idle";
    }
}

// ── Agent root row ───────────────────────────────────────────────────────

function AgentRow({
    node,
    focusedBlockId,
}: {
    node: AgentTreeNode;
    focusedBlockId: () => string | null;
}): JSX.Element {
    const displayStatus = createMemo<AgentDisplayStatus>(() =>
        phaseToDisplayStatus(node.blockId, node.agentStatus)
    );

    return (
        <div class="swarm-agent-group">
            <div
                classList={{
                    "swarm-agent-row": true,
                    [`swarm-agent-row--${node.agentStatus}`]: true,
                    "swarm-agent-row--active": focusedBlockId() === node.blockId,
                }}
                onClick={() => node.blockId && void focusBlock(node.blockId)}
                title={node.agentName}
            >
                <span class="swarm-agent-icon">
                    <ProviderLogo provider={node.agentProvider ?? "agentmux"} size={16} />
                </span>
                <span class="swarm-agent-label">{node.agentName}</span>
                <AgentStatusChip status={displayStatus()} />
            </div>
            <div class="swarm-children">
                <Show when={node.activitySummary}>
                    <div class="swarm-activity-summary">{node.activitySummary}</div>
                </Show>
                <For each={node.subagents}>
                    {(sub) => <SubagentRow sub={sub} />}
                </For>
            </div>
        </div>
    );
}

// ── Subagent child row ───────────────────────────────────────────────────

function SubagentRow({ sub }: { sub: ActiveSubagent }): JSX.Element {
    const handleOpen = () => {
        if (isSubagentPaneOpen(sub.agent_id)) return;
        void openSubagentPane({
            subagentId: sub.agent_id,
            slug: sub.slug,
            parentAgent: sub.parent_agent,
            parentBlockId: sub.parent_block_id,
            sessionId: sub.session_id,
        });
    };

    return (
        <div
            class={`swarm-subagent-row swarm-subagent-row--${sub.status}`}
            onClick={handleOpen}
            title={sub.slug || sub.agent_id}
        >
            <span class="swarm-subagent-slug">{sub.slug || sub.agent_id.substring(0, 7)}</span>
            <AgentStatusChip status={sub.status === "active" ? "working" : "idle"} />
        </div>
    );
}

// ── Status chip ──────────────────────────────────────────────────────────

const STATUS_LABEL: Record<AgentDisplayStatus, string> = {
    working:      "working",
    tools:        "tools",
    stopping:     "stopping",
    idle:         "idle",
    error:        "error",
    disconnected: "offline",
};

function AgentStatusChip({ status }: { status: AgentDisplayStatus }): JSX.Element {
    return (
        <span class={`swarm-status-chip swarm-status-chip--${status}`}>
            <span class={`swarm-status-dot swarm-status-dot--${status}`} />
            {STATUS_LABEL[status]}
        </span>
    );
}
