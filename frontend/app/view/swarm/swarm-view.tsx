// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, createEffect, onCleanup, For, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel, AgentTreeNode, ActiveSubagent, ActiveShell, ActiveCron, WorkflowDispatch, SubagentEvent, DispatchActivityEntry } from "./swarm-model";
import { subagentDisplayLabel, subagentRowKey } from "./swarm-model";
import { ProviderLogo } from "@/app/element/ProviderLogo";
import { callBackendService } from "@/store/wos";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS, workspace, setActiveTab, atoms, getApi } from "@/app/store/global";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { getBlockTurnPhase } from "@/app/store/agentActivity";
import { WorkspaceService } from "@/app/store/services";
import { recordTurn } from "@/app/store/token-usage";
import { useTick } from "@/app/hook/useTick";
import { formatCompactNumber } from "@/util/format-count";
import { formatElapsedClock } from "@/util/format-time";
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

export type AgentDisplayStatus = "working" | "tools" | "stopping" | "idle" | "error" | "disconnected" | "unknown" | "interrupted";

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

/**
 * A subagent runs inside its parent agent's own CLI process — a Task-tool
 * call is synchronous within the parent's turn — so it cannot genuinely
 * still be active once the parent's own turn has ended. The backend already
 * reconciles this (SubagentStatus::Abandoned), but currently only at pane
 * reopen (see docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md
 * Open Question 1) — this is a client-side backstop using the exact same
 * signal (`parentAgentStatus`, fed by GetControllerStatus/controllerstatus,
 * the same turn_active the backend's reconcile_stale_subagents reads) for
 * the mid-session gap until real-time backend reconciliation ships. Purely
 * a DISPLAY decision — never mutates the underlying ActiveSubagent, so
 * grouping (activeCount/retired) keeps reading the real backend status.
 */
export function subagentDisplayStatus(sub: ActiveSubagent, parentAgentStatus: "running" | "idle"): AgentDisplayStatus {
    if (sub.status === "abandoned") return "interrupted";
    if (sub.status === "active") {
        return parentAgentStatus === "idle" ? "interrupted" : "working";
    }
    return "idle"; // completed
}

// ── Agent root row ───────────────────────────────────────────────────────

const fmtCtx = formatCompactNumber;

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
    const collapsed = createMemo(() => model.isAgentCollapsed(node.blockId));
    const totalRows = createMemo(
        () => node.agentToolRows.length + node.workflowRows.length + node.shellRows.length + node.cronRows.length
    );
    const hasChildren = createMemo(() => totalRows() > 0);

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
                onClick={() => node.blockId && model.toggleAgentCollapsed(node.blockId)}
                title={node.agentName}
            >
                <div class="swarm-agent-row">
                    {/* Chevron only when there's a subtree to collapse; a
                        fixed-width spacer otherwise so labels stay aligned
                        across rows with and without children. No own click
                        handler — the enclosing card's click already toggles. */}
                    <Show
                        when={hasChildren()}
                        fallback={<span class="swarm-agent-expand-spacer" />}
                    >
                        <i class={`fa-solid fa-${collapsed() ? "chevron-right" : "chevron-down"} swarm-agent-expand-icon`} />
                    </Show>
                    <span class="swarm-agent-icon">
                        <ProviderLogo provider={node.agentProvider ?? "agentmux"} size={16} />
                    </span>
                    <span class="swarm-agent-label">{node.agentName}</span>
                    <Show when={collapsed() && hasChildren()}>
                        <span class="swarm-agent-collapsed-count">{totalRows()}</span>
                    </Show>
                    <Show when={node.contextTokens != null}>
                        <span class="swarm-ctx-size">{fmtCtx(node.contextTokens!)}</span>
                    </Show>
                    <AgentStatusChip status={displayStatus()} />
                    <button
                        class="swarm-agent-focus"
                        title="Focus"
                        onClick={(e) => {
                            e.stopPropagation();
                            if (node.blockId) void focusBlock(node.blockId);
                        }}
                    >
                        <i class="fa-solid fa-arrow-up-right-from-square" />
                    </button>
                </div>
                <Show when={node.activitySummary}>
                    <div classList={{ "swarm-activity-summary": true, "swarm-activity-summary--flash": summaryFlash() }}>
                        {node.activitySummary}
                    </div>
                </Show>
            </div>
            <Show when={!collapsed()}>
                <div class="swarm-children">
                    <AgentToolBucket rows={node.agentToolRows} model={model} parentAgentStatus={node.agentStatus} />
                    <WorkflowBucket rows={node.workflowRows} model={model} />
                    <ShellBucket rows={node.shellRows} />
                    <CronBucket rows={node.cronRows} />
                </div>
            </Show>
        </div>
    );
}

// ── Two fixed dispatch-kind buckets (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_
//    MODEL_2026_07_19 §4) — always exactly these two, never data-driven
//    groups; each hides entirely when empty (no "always visible" precedent
//    exists anywhere else in this codebase, see that spec's §6). ─────────

function AgentToolBucket({
    rows,
    model,
    parentAgentStatus,
}: {
    rows: ActiveSubagent[];
    model: SwarmViewModel;
    parentAgentStatus: "running" | "idle";
}): JSX.Element {
    return (
        <Show when={rows.length > 0}>
            <div class="swarm-bucket swarm-bucket--agent-tool">
                <div class="swarm-bucket-header">
                    <span class="swarm-bucket-label">Agent Tool</span>
                    <span class="swarm-bucket-count">{rows.length}</span>
                </div>
                <For each={rows}>
                    {(sub) => <SubagentRow sub={sub} model={model} parentAgentStatus={parentAgentStatus} />}
                </For>
            </div>
        </Show>
    );
}

function WorkflowBucket({ rows, model }: { rows: WorkflowDispatch[]; model: SwarmViewModel }): JSX.Element {
    return (
        <Show when={rows.length > 0}>
            <div class="swarm-bucket swarm-bucket--workflow">
                <div class="swarm-bucket-header">
                    <span class="swarm-bucket-label">Workflow</span>
                    <span class="swarm-bucket-count">{rows.length}</span>
                </div>
                <For each={rows}>{(group) => <WorkflowDispatchRow group={group} model={model} />}</For>
            </div>
        </Show>
    );
}

// Shell bucket — Phase 1 of SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20.
// Appended after Agent Tool/Workflow (spec's resolved open question 1):
// those two are semantically-named, agent-level units of work; shells are
// lower-level/mechanical (raw commands), so most-abstract-first reads best.
// No expand-to-feed like the other two buckets — Phase 1 is deliberately
// flat (title + elapsed + stop), matching the "good starting point" scope;
// showing live shell output here is a larger follow-up, not attempted now.
function ShellBucket({ rows }: { rows: ActiveShell[] }): JSX.Element {
    return (
        <Show when={rows.length > 0}>
            <div class="swarm-bucket swarm-bucket--shell">
                <div class="swarm-bucket-header">
                    <span class="swarm-bucket-label">Shell</span>
                    <span class="swarm-bucket-count">{rows.length}</span>
                </div>
                <For each={rows}>{(shell) => <ShellRow shell={shell} />}</For>
            </div>
        </Show>
    );
}

function ShellRow({ shell }: { shell: ActiveShell }): JSX.Element {
    const tick = useTick(1000);
    const elapsed = createMemo(() => {
        tick();
        return formatElapsedClock(Date.now() - shell.started_at);
    });

    const handleStop = (e: MouseEvent) => {
        e.stopPropagation();
        RpcApi.ShellStopCommand(TabRpcClient, { shell_id: shell.shell_id }).catch(() => {
            // best-effort — the exit event (shell_chunk op:"exit") reconciles
            // the active list via scheduleLoadShells either way
        });
    };

    return (
        <div class="swarm-shell-row" title={shell.cmd}>
            <span class="swarm-shell-title">{shell.title}</span>
            <span class="swarm-shell-elapsed">{elapsed()}</span>
            <button class="swarm-shell-stop" title="Stop" onClick={handleStop}>
                <i class="fa-solid fa-stop" />
            </button>
        </div>
    );
}

// Cron bucket — Phase 2 of SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20.
// Appended last (after Shell) — the spec's resolved ordering question is
// most-abstract-first; a recurring scheduled job sits at the same
// "mechanical" tier as a shell, so it's grouped alongside rather than
// interleaved with Agent Tool/Workflow. Flat rows, same shape as
// ShellBucket — no pause/resume/delete action here; Phase 2 is read-only
// visibility, matching the spec's stated row content (no action button).
function CronBucket({ rows }: { rows: ActiveCron[] }): JSX.Element {
    return (
        <Show when={rows.length > 0}>
            <div class="swarm-bucket swarm-bucket--cron">
                <div class="swarm-bucket-header">
                    <span class="swarm-bucket-label">Cron</span>
                    <span class="swarm-bucket-count">{rows.length}</span>
                </div>
                <For each={rows}>{(cron) => <CronRow cron={cron} />}</For>
            </div>
        </Show>
    );
}

function formatLastFired(unixSec: number | null): string {
    if (unixSec == null) return "never fired";
    const deltaMin = Math.floor((Date.now() - unixSec * 1000) / 60_000);
    if (deltaMin < 1) return "just now";
    if (deltaMin < 60) return `${deltaMin}m ago`;
    const deltaHr = Math.floor(deltaMin / 60);
    if (deltaHr < 24) return `${deltaHr}h ago`;
    return `${Math.floor(deltaHr / 24)}d ago`;
}

function CronRow({ cron }: { cron: ActiveCron }): JSX.Element {
    // 60s tick — "last fired" doesn't need per-second granularity like a
    // shell's live elapsed counter.
    const tick = useTick(60_000);
    const lastFired = createMemo(() => {
        tick();
        return formatLastFired(cron.last_fired);
    });
    const fireCountLabel = createMemo(() =>
        cron.max_fires != null ? `${cron.fire_count}/${cron.max_fires}` : `${cron.fire_count}`
    );

    return (
        <div class="swarm-cron-row" title={`${cron.expression} → ${cron.target}`}>
            <span class="swarm-cron-name">{cron.name}</span>
            <span class="swarm-cron-expression">{cron.expression}</span>
            <span class="swarm-cron-last-fired">{lastFired()}</span>
            <span class="swarm-cron-fire-count">{fireCountLabel()}</span>
            <span class={`swarm-cron-status-badge swarm-cron-status-badge--${cron.enabled ? "active" : "paused"}`}>
                {cron.enabled ? "Active" : "Paused"}
            </span>
        </div>
    );
}

// ── Workflow dispatch row (collapsed by default) ────────────────────────

/**
 * One row for a Workflow-kind `AgentDispatch`. SPEC_AGENT_DISPATCH_SUBAGENT_
 * HIERARCHY_2026_07_17 §7: never one row per member, however many the
 * dispatch has (this session's own crash retro found one workflow run with
 * 1,030+ members). Expanding shows a concatenated activity feed
 * (`DispatchActivityFeed`) instead of nested `SubagentRow`s.
 */
function WorkflowDispatchRow({
    group,
    model,
}: {
    group: WorkflowDispatch;
    model: SwarmViewModel;
}): JSX.Element {
    // Expand state lives on the ViewModel, not a local signal — see
    // SwarmViewModel._expandedIds for why a local signal here silently
    // collapses on unrelated tree refreshes.
    const expanded = createMemo(() => model.isExpanded(group.dispatchId));
    const activeCount = createMemo(() => group.memberCount - group.membersDone);

    // `group.status === "retired"` here is the pre-existing "every member
    // done" label (SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17) — NOT
    // the same concept as the user-driven Retire action below (SPEC_
    // SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6), just a
    // same-word coincidence. Retire is only offered once a dispatch is
    // ALREADY (label-)retired — nothing to dismiss on one still running.
    const handleRetire = (e: MouseEvent) => {
        e.stopPropagation();
        model.retireRow(group.dispatchId, group.lastEventAt);
    };

    return (
        <div class={`swarm-workflow-group swarm-workflow-group--${group.status}`}>
            <div
                class="swarm-workflow-header"
                onClick={() => model.toggleDispatchExpanded(group.dispatchId)}
                title={group.name}
            >
                <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-workflow-expand-icon`} />
                <span class="swarm-workflow-name">{group.name}</span>
                <span class="swarm-workflow-count">
                    {group.status === "active"
                        ? `${activeCount()}/${group.memberCount} active`
                        : `${group.memberCount} retired`}
                </span>
                <span class={`swarm-workflow-status-badge swarm-workflow-status-badge--${group.status}`}>
                    {group.status === "active" ? "Active" : "Retired"}
                </span>
                <Show when={group.status === "retired"}>
                    <button class="swarm-subagent-retire" title="Retire" onClick={handleRetire}>
                        <i class="fa-solid fa-xmark" />
                    </button>
                </Show>
            </div>
            <Show when={expanded()}>
                <DispatchActivityFeed rowKey={group.dispatchId} dispatchId={group.dispatchId} model={model} />
            </Show>
        </div>
    );
}

/**
 * Concatenated activity feed for an expanded Agent Tool or Workflow row —
 * one chronological, member-tagged stream instead of nested member rows,
 * fed by `createDispatchDetail`'s `dispatch:activity` subscription. A solo
 * (Agent Tool) dispatch also backfills history via `GetHistory`; a Workflow
 * dispatch stays live-only (see `createDispatchDetail`'s doc comment for
 * why) — so an empty feed right after expand is expected for a Workflow row
 * and doesn't necessarily mean "no history", hence the kind-neutral copy.
 */
function DispatchActivityFeed({
    rowKey,
    dispatchId,
    model,
    backfillAgentId,
}: {
    /** Cache/expand-state identity for THIS row — must be unique per row.
     *  For a `WorkflowDispatchRow` this is the same as `dispatchId` (a
     *  workflow's own dispatch_id is always 1:1 with its row). For an Agent
     *  Tool row it must NOT be `dispatchId`: an orphaned workflow member (a
     *  real "wf_..." dispatch_id shared by every sibling member still
     *  waiting on a stale/lagging `ListDispatches`) would otherwise collide
     *  every sibling row onto the same cached `DispatchDetail` and the same
     *  expand-state entry (reagent P1 on #2232) — pass a per-agent key like
     *  `agent:${sub.agent_id}` instead, which is always unique. */
    rowKey: string;
    /** The dispatch_id to subscribe to for live `dispatch:activity` events —
     *  the REAL dispatch_id, which may legitimately be shared with sibling
     *  rows (the orphaned-member case above); `backfillAgentId` (below)
     *  scopes both the live feed and the backfill down to one agent when
     *  that's the case. */
    dispatchId: string;
    model: SwarmViewModel;
    /** Pass the row's own single subagent's `agent_id` for an Agent Tool row
     *  (including an orphaned workflow member — see `getDispatchDetail`'s
     *  doc comment in swarm-model.ts); omit for a genuine multi-member
     *  `WorkflowDispatchRow`. */
    backfillAgentId?: string;
}): JSX.Element {
    const detail = model.getDispatchDetail(rowKey, dispatchId, backfillAgentId);
    const entries = detail.entriesAtom;
    return (
        <div class="swarm-dispatch-feed">
            <Show when={entries().length === 0}>
                <div class="swarm-dispatch-feed-empty">No activity yet.</div>
            </Show>
            <For each={entries()}>{(entry) => <DispatchActivityFeedEntry entry={entry} />}</For>
        </div>
    );
}

function DispatchActivityFeedEntry({ entry }: { entry: DispatchActivityEntry }): JSX.Element {
    const et = entry.event.event_type;
    const [expanded, setExpanded] = createSignal(false);
    const tag = <span class="swarm-dispatch-feed-tag">{entry.agentId.substring(0, 7)}</span>;

    switch (et.type) {
        case "text":
        case "result":
            return (
                <div class="swarm-dispatch-feed-entry">
                    {tag}
                    <pre class="swarm-subagent-detail-text">{et.content}</pre>
                </div>
            );
        case "tool_use":
            return (
                <div class="swarm-dispatch-feed-entry">
                    {tag}
                    <div class="swarm-subagent-detail-tool">
                        <div class="swarm-subagent-detail-tool-header" onClick={() => setExpanded(!expanded())}>
                            <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-subagent-detail-expand-icon`} />
                            <span class="swarm-subagent-detail-tool-name">{et.name}</span>
                        </div>
                        <Show when={expanded()}>
                            <pre class="swarm-subagent-detail-text">{et.input_summary}</pre>
                        </Show>
                    </div>
                </div>
            );
        case "tool_result":
            return (
                <div class="swarm-dispatch-feed-entry">
                    {tag}
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
                </div>
            );
        case "progress":
            return (
                <div class="swarm-dispatch-feed-entry">
                    {tag}
                    <div class="swarm-subagent-detail-progress">
                        <i class="fa-solid fa-spinner fa-spin" />
                        <span>{et.output}</span>
                    </div>
                </div>
            );
        default:
            return null;
    }
}

// ── Agent Tool (solo dispatch) row ──────────────────────────────────────

function SubagentRow({
    sub,
    model,
    parentAgentStatus,
}: {
    sub: ActiveSubagent;
    model: SwarmViewModel;
    parentAgentStatus: "running" | "idle";
}): JSX.Element {
    // Unified with WorkflowDispatchRow onto the same expandedIds/
    // dispatchDetailCache mechanism (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_
    // MODEL_2026_07_19 §4), but keyed by `agent_id`, NOT `dispatch_id`: an
    // orphaned workflow member's `dispatch_id` is a real, possibly-shared
    // "wf_..." id (every sibling still waiting on a stale/lagging
    // `ListDispatches` shares it) — keying expand-state/cache off it would
    // collide every sibling row's expand toggle and cached feed onto the
    // first one (reagent P1 on #2232). `agent_id` is always unique per row.
    const rowKey = createMemo(() => subagentRowKey(sub.agent_id));
    const expanded = createMemo(() => model.isExpanded(rowKey()));
    // See subagentDisplayLabel's doc comment (swarm-model.ts) — as of Phase A
    // this resolves eagerly at dispatch time for the common case; the
    // slug/agent_id fallback chain only matters for the brief window before
    // that resolves (or if it never does).
    const displayLabel = createMemo(() => subagentDisplayLabel(sub));

    const handleToggle = () => {
        const wasExpanded = expanded();
        model.toggleDispatchExpanded(rowKey());
        if (!wasExpanded && !sub.display_name) {
            // Fallback safety net — eager naming (Phase A) should already
            // have resolved this by the time a user gets here, but fire the
            // on-demand call too in case it hasn't (still in flight, or
            // failed). Fire-and-forget — the row's label picks up the name
            // via the subagent:named event (swarm-model.ts), not this call's
            // return; we only need the return here for cost accounting.
            void callBackendService("subagent", "GenerateName", [sub.agent_id]).then((result: any) => {
                if (result?.tokens) recordTurn("ambient:subagent_name", result.tokens);
            });
        }
    };

    const displayStatus = createMemo(() => subagentDisplayStatus(sub, parentAgentStatus));

    // Row/group dimming must track the same effective status as the chip
    // (reagent P2 on #2134) — otherwise a client-side "interrupted" backstop
    // shows the new chip/dot but the row stays full-opacity, since only a
    // backend-confirmed sub.status === "abandoned" would dim it directly.
    const dimVariant = createMemo(() => {
        const status = displayStatus();
        if (status === "interrupted") return "abandoned";
        if (status === "idle") return "completed";
        return "active";
    });

    // Retire (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 §6):
    // only a terminal-status row has anything to dismiss — a still-"working"
    // row is legitimately in progress, nothing to retire yet.
    const canRetire = createMemo(() => displayStatus() === "idle" || displayStatus() === "interrupted");
    const handleRetire = (e: MouseEvent) => {
        e.stopPropagation();
        model.retireRow(rowKey(), sub.last_event_at);
    };

    return (
        <div class={`swarm-subagent-group swarm-subagent-group--${dimVariant()}`}>
            <div
                class={`swarm-subagent-row swarm-subagent-row--${dimVariant()}`}
                onClick={handleToggle}
                title={displayLabel()}
            >
                <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} swarm-subagent-expand-icon`} />
                <span class="swarm-subagent-slug">{displayLabel()}</span>
                <AgentStatusChip status={displayStatus()} />
                <Show when={canRetire()}>
                    <button class="swarm-subagent-retire" title="Retire" onClick={handleRetire}>
                        <i class="fa-solid fa-xmark" />
                    </button>
                </Show>
            </div>
            <Show when={expanded()}>
                <DispatchActivityFeed
                    rowKey={rowKey()}
                    dispatchId={sub.dispatch_id}
                    model={model}
                    backfillAgentId={sub.agent_id}
                />
            </Show>
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
    unknown:      "unknown",
    interrupted:  "interrupted",
};

function AgentStatusChip({ status }: { status: AgentDisplayStatus }): JSX.Element {
    return (
        <span class={`swarm-status-chip swarm-status-chip--${status}`}>
            <span class={`swarm-status-dot swarm-status-dot--${status}`} />
            {STATUS_LABEL[status]}
        </span>
    );
}
