// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, createEffect, onCleanup, For, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel, AgentTreeNode, ActiveSubagent, WorkflowGroup, NameGroup, SubagentDetail, SubagentEvent } from "./swarm-model";
import { isWorkflowGroup, isNameGroup, groupCacheKey } from "./swarm-model";
import { ProviderLogo } from "@/app/element/ProviderLogo";
import { callBackendService } from "@/store/wos";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS, workspace, setActiveTab, atoms, getApi } from "@/app/store/global";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { getBlockTurnPhase } from "@/app/store/agentActivity";
import { WorkspaceService } from "@/app/store/services";
import { recordTurn } from "@/app/store/token-usage";
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
    // We need Tab.blockids to find which tab holds the block. Layout models for
    // other-window tabs are not available from this renderer. We fetch each Tab
    // from the server (cache hit is instant; miss triggers one server round-trip).
    // focusNode is intentionally omitted: a layout model in another renderer
    // process cannot be driven from this side.
    const allWorkspaces = await RpcApi.WorkspaceListCommand(TabRpcClient);
    for (const wsInfo of allWorkspaces) {
        if (wsInfo.workspacedata.oid === ws?.oid) continue;
        const wsData = wsInfo.workspacedata;
        const allTabIds = [...(wsData.pinnedtabids ?? []), ...(wsData.tabids ?? [])];
        for (const tabId of allTabIds) {
            const oref = WOS.makeORef("tab", tabId);
            // Use cached value when available; reloadWaveObject on cache miss.
            const cached = WOS.getObjectValue<Tab>(oref);
            const tab = cached ?? (await WOS.reloadWaveObject<Tab>(oref));
            if (!tab?.blockids?.includes(blockId)) continue;
            await WorkspaceService.SetActiveTab(wsData.oid, tabId);
            const instances = await getApi().listWindowInstances();
            const instance = instances.find((i) => i.windowId === wsInfo.windowid);
            if (instance?.label) {
                await getApi().focusWindow(instance.label);
            }
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
                            {(node) => <AgentRow node={node} focusedBlockId={focusedBlockId} model={model} />}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

// ── Status derived from TurnPhase ────────────────────────────────────────

type AgentDisplayStatus = "working" | "tools" | "stopping" | "idle" | "error" | "disconnected" | "unknown";

function phaseToDisplayStatus(blockId: string, fallback: "running" | "idle"): AgentDisplayStatus {
    const phaseAccessor = getBlockTurnPhase(blockId);
    // A block only has a registered TurnPhase while its pane is mounted in
    // THIS renderer (registerActivity() in agentActivity.ts runs from the
    // mounted agent-pane component). A subagent running in an unmounted
    // pane, a background tab, or another workspace has no entry here — that
    // does NOT mean it's idle, it means we have no visibility into it from
    // here. Conflating the two used to render active agents as flatly
    // "idle" ("nothing in progress" when something clearly was) — see
    // docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §1.4/§3.5.
    //
    // `fallback` is `node.agentStatus` — backend-verified via
    // `GetControllerStatus`/`ControllerStatus` (swarm-model.ts's
    // `derivedRunningStatus`), independent of whether this renderer has a
    // mounted pane for the block. For agent panes it's turn-precise (keyed
    // off `turn_active`, not raw process-alive `shellprocstatus` — see
    // docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    // Finding 1), so it's safe to trust here instead of falling through to
    // "unknown" for an unmounted/background agent.
    if (!phaseAccessor) return fallback === "running" ? "working" : "idle";
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

function fmtCtx(tokens: number): string {
    return `${Math.round(tokens / 100) / 10}k`;
}

function AgentRow({
    node,
    focusedBlockId,
    model,
}: {
    node: AgentTreeNode;
    focusedBlockId: () => string | null;
    model: SwarmViewModel;
}): JSX.Element {
    const displayStatus = createMemo<AgentDisplayStatus>(() =>
        phaseToDisplayStatus(node.blockId, node.agentStatus)
    );

    const [summaryFlash, setSummaryFlash] = createSignal(false);
    let flashTimer: ReturnType<typeof setTimeout> | undefined;
    let prevSummary: string | null = node.activitySummary; // don't flash existing summary on mount
    createEffect(() => {
        const summary = node.activitySummary;
        if (summary && summary !== prevSummary) {
            clearTimeout(flashTimer);
            setSummaryFlash(true);
            flashTimer = setTimeout(() => setSummaryFlash(false), 600);
        }
        prevSummary = summary;
    });
    onCleanup(() => clearTimeout(flashTimer));

    return (
        <div class="swarm-agent-group">
            <div
                classList={{
                    "swarm-agent-card": true,
                    [`swarm-agent-card--${node.agentStatus}`]: true,
                    "swarm-agent-card--active": focusedBlockId() === node.blockId,
                }}
                onClick={() => node.blockId && void focusBlock(node.blockId)}
                title={node.agentName}
            >
                <div class="swarm-agent-row">
                    <span class="swarm-agent-icon">
                        <ProviderLogo provider={node.agentProvider ?? "agentmux"} size={16} />
                    </span>
                    <span class="swarm-agent-label">{node.agentName}</span>
                    <Show when={node.contextTokens != null}>
                        <span class="swarm-ctx-size">{fmtCtx(node.contextTokens!)}</span>
                    </Show>
                    <AgentStatusChip status={displayStatus()} />
                </div>
                <Show when={node.activitySummary}>
                    <div classList={{ "swarm-activity-summary": true, "swarm-activity-summary--flash": summaryFlash() }}>
                        {node.activitySummary}
                    </div>
                </Show>
            </div>
            <div class="swarm-children">
                <For each={node.subagents}>
                    {(child) => isWorkflowGroup(child)
                        ? <WorkflowGroupRow group={child} model={model} />
                        : isNameGroup(child)
                        ? <NameGroupRow group={child} model={model} />
                        : <SubagentRow sub={child} model={model} />}
                </For>
            </div>
        </div>
    );
}

// ── Workflow group row (collapsed by default) ───────────────────────────

function WorkflowGroupRow({ group, model }: { group: WorkflowGroup; model: SwarmViewModel }): JSX.Element {
    // Expand state lives on the ViewModel, not a local signal — see
    // SwarmViewModel._expandedIds for why a local signal here silently
    // collapses on unrelated tree refreshes.
    const expanded = createMemo(() => model.isExpanded(group.workflowId));

    return (
        <div class={`swarm-workflow-group swarm-workflow-group--${group.status}`}>
            <div
                class="swarm-workflow-header"
                onClick={() => model.toggleExpanded(group.workflowId)}
                title={group.name}
            >
                <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-workflow-expand-icon`} />
                <span class="swarm-workflow-name">{group.name}</span>
                <span class="swarm-workflow-count">
                    {group.status === "active" ? `${group.activeCount}/${group.totalCount} active` : `${group.totalCount} retired`}
                </span>
                <span class={`swarm-workflow-status-badge swarm-workflow-status-badge--${group.status}`}>
                    {group.status === "active" ? "Active" : "Retired"}
                </span>
            </div>
            <Show when={expanded()}>
                <div class="swarm-workflow-members">
                    <For each={group.subagents}>
                        {(sub) => <SubagentRow sub={sub} model={model} />}
                    </For>
                </div>
            </Show>
        </div>
    );
}

// ── Name group row (loose subagents sharing one display_name) ──────────

function NameGroupRow({ group, model }: { group: NameGroup; model: SwarmViewModel }): JSX.Element {
    // Same expand-state-on-the-ViewModel rationale as WorkflowGroupRow —
    // reuses groupCacheKey's "name:<name>" namespacing so this can never
    // collide with a WorkflowGroupRow's workflowId or a SubagentRow's
    // agent_id in the shared expandedIds set.
    const key = groupCacheKey(group);
    const expanded = createMemo(() => model.isExpanded(key));

    return (
        <div class={`swarm-workflow-group swarm-workflow-group--${group.status}`}>
            <div
                class="swarm-workflow-header"
                onClick={() => model.toggleExpanded(key)}
                title={group.name}
            >
                <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-workflow-expand-icon`} />
                <span class="swarm-workflow-name">{group.name}</span>
                <span class="swarm-workflow-count">
                    {group.status === "active" ? `${group.activeCount}/${group.totalCount} active` : `${group.totalCount} retired`}
                </span>
                <span class={`swarm-workflow-status-badge swarm-workflow-status-badge--${group.status}`}>
                    {group.status === "active" ? "Active" : "Retired"}
                </span>
            </div>
            <Show when={expanded()}>
                <div class="swarm-workflow-members">
                    <For each={group.subagents}>
                        {(sub) => <SubagentRow sub={sub} model={model} />}
                    </For>
                </div>
            </Show>
        </div>
    );
}

// ── Subagent child row ───────────────────────────────────────────────────

function SubagentRow({ sub, model }: { sub: ActiveSubagent; model: SwarmViewModel }): JSX.Element {
    const expanded = createMemo(() => model.isExpanded(sub.agent_id));
    const displayLabel = createMemo(() => sub.display_name || sub.slug || sub.agent_id.substring(0, 7));

    const handleToggle = () => {
        const wasExpanded = expanded();
        model.toggleSubagentExpanded(sub.agent_id);
        if (!wasExpanded && !sub.display_name) {
            // Fire-and-forget — the row's label/detail header picks up the
            // name via the subagent:named event (swarm-model.ts), not this
            // call's return; we only need the return here for cost accounting.
            void callBackendService("subagent", "GenerateName", [sub.agent_id]).then((result: any) => {
                if (result?.tokens) recordTurn("ambient:subagent_name", result.tokens);
            });
        }
    };

    return (
        <div class={`swarm-subagent-group swarm-subagent-group--${sub.status}`}>
            <div
                class={`swarm-subagent-row swarm-subagent-row--${sub.status}`}
                onClick={handleToggle}
                title={displayLabel()}
            >
                <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-subagent-expand-icon`} />
                <span class="swarm-subagent-slug">{displayLabel()}</span>
                <AgentStatusChip status={sub.status === "active" ? "working" : "idle"} />
            </div>
            <Show when={expanded()}>
                <SubagentDetailPane sub={sub} detail={model.getSubagentDetail(sub.agent_id)} />
            </Show>
        </div>
    );
}

// ── Subagent inline detail (expanded event log) ──────────────────────────

function SubagentDetailPane({ sub, detail }: { sub: ActiveSubagent; detail: SubagentDetail }): JSX.Element {
    const info = detail.infoAtom;
    const events = detail.eventsAtom;
    const status = detail.statusAtom;

    const name = createMemo(() =>
        info()?.display_name || sub.display_name || info()?.slug || sub.slug || sub.agent_id.substring(0, 7)
    );
    const modelName = createMemo(() => info()?.model ?? sub.model);
    const eventCount = createMemo(() => info()?.event_count ?? sub.event_count);

    return (
        <div class="swarm-subagent-detail">
            <div class="swarm-subagent-detail-header">
                <span class="swarm-subagent-detail-name">{name()}</span>
                <span class="swarm-subagent-detail-meta">{sub.agent_id.substring(0, 7)}</span>
                <span class="swarm-subagent-detail-meta">{eventCount()} events</span>
                <Show when={modelName()}>
                    <span class="swarm-subagent-detail-meta">{modelName()}</span>
                </Show>
                <span class="swarm-subagent-detail-meta">{sub.parent_agent}</span>
            </div>
            <div class="swarm-subagent-detail-log">
                <Show when={status() === "loading"}>
                    <div class="swarm-subagent-detail-loading">Loading…</div>
                </Show>
                <Show when={events().length === 0 && status() !== "loading"}>
                    <div class="swarm-subagent-detail-empty">No activity yet</div>
                </Show>
                <For each={events()}>{(event) => <SubagentDetailEvent event={event} />}</For>
            </div>
        </div>
    );
}

function SubagentDetailEvent({ event }: { event: SubagentEvent }): JSX.Element {
    const et = event.event_type;
    const [expanded, setExpanded] = createSignal(false);

    switch (et.type) {
        case "text":
        case "result":
            return <pre class="swarm-subagent-detail-text">{et.content}</pre>;
        case "tool_use":
            return (
                <div class="swarm-subagent-detail-tool">
                    <div class="swarm-subagent-detail-tool-header" onClick={() => setExpanded(!expanded())}>
                        <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-subagent-detail-expand-icon`} />
                        <span class="swarm-subagent-detail-tool-name">{et.name}</span>
                    </div>
                    <Show when={expanded()}>
                        <pre class="swarm-subagent-detail-text">{et.input_summary}</pre>
                    </Show>
                </div>
            );
        case "tool_result":
            return (
                <div class="swarm-subagent-detail-tool">
                    <div
                        class={`swarm-subagent-detail-tool-header ${et.is_error ? "swarm-subagent-detail-tool-header--error" : ""}`}
                        onClick={() => setExpanded(!expanded())}
                    >
                        <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-subagent-detail-expand-icon`} />
                        <span>{et.is_error ? "Error" : "Result"}</span>
                    </div>
                    <Show when={expanded()}>
                        <pre class={`swarm-subagent-detail-text ${et.is_error ? "swarm-subagent-detail-text--error" : ""}`}>
                            {et.preview}
                        </pre>
                    </Show>
                </div>
            );
        case "progress":
            return (
                <div class="swarm-subagent-detail-progress">
                    <i class="fa-solid fa-spinner fa-spin" />
                    <span>{et.output}</span>
                </div>
            );
        default:
            return null;
    }
}

// ── Status chip ──────────────────────────────────────────────────────────

const STATUS_LABEL: Record<AgentDisplayStatus, string> = {
    working:      "working",
    tools:        "tools",
    stopping:     "stopping",
    idle:         "idle",
    error:        "error",
    disconnected: "offline",
    unknown:      "unknown",
};

function AgentStatusChip({ status }: { status: AgentDisplayStatus }): JSX.Element {
    return (
        <span class={`swarm-status-chip swarm-status-chip--${status}`}>
            <span class={`swarm-status-dot swarm-status-dot--${status}`} />
            {STATUS_LABEL[status]}
        </span>
    );
}
