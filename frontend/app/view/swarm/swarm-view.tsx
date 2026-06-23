// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel, AgentTreeNode, ActiveSubagent } from "./swarm-model";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS, workspace, setActiveTab } from "@/app/store/global";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import "./swarm-view.scss";

// Navigate to the pane for a given block ID, switching tabs if needed.
// Uses layout models (always cached for visited tabs) rather than Tab.blockids
// (which requires the Tab object to be in the WOS cache).
async function focusBlock(blockId: string): Promise<void> {
    const ws = workspace();
    if (!ws) return;
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
                            {(node) => <AgentRow node={node} />}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

// ── Agent root row ───────────────────────────────────────────────────────

function AgentRow({ node }: { node: AgentTreeNode }): JSX.Element {
    return (
        <div class="swarm-agent-group">
            <div
                class={`swarm-agent-row swarm-agent-row--${node.agentStatus}`}
                onClick={() => void focusBlock(node.blockId)}
                title={node.agentName}
            >
                <span class="swarm-agent-icon">⬡</span>
                <span class="swarm-agent-label">{node.agentName}</span>
                <StatusChip status={node.agentStatus === "running" ? "running" : "idle"} />
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
            <StatusChip status={sub.status === "active" ? "running" : "done"} />
        </div>
    );
}

// ── Status chip ──────────────────────────────────────────────────────────

function StatusChip({ status }: { status: "running" | "idle" | "done" }): JSX.Element {
    const label = status === "running" ? "running" : status === "done" ? "done" : "idle";
    return (
        <span class={`swarm-status-chip swarm-status-chip--${status}`}>
            <span class={`swarm-status-dot swarm-status-dot--${status}`} />
            {label}
        </span>
    );
}
