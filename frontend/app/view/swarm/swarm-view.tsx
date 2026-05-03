// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { SwarmViewModel, ActiveSubagent, SwarmEntry } from "./swarm-model";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS } from "@/app/store/global";
import "./swarm-view.scss";

export function SwarmView(props: ViewComponentProps<SwarmViewModel>): JSX.Element {
    const model = props.model;
    const block = WOS.getWaveObjectAtom<Block>(`block:${model.blockId}`);

    // Per-pane zoom — same `term:zoom` meta key as terminal/agent panes.
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
                ev.stopPropagation();
                setZoom(zoomFactor() + STEP);
            } else if (ev.key === "-" || ev.key === "_") {
                ev.preventDefault();
                ev.stopPropagation();
                setZoom(zoomFactor() - STEP);
            } else if (ev.key === "0") {
                ev.preventDefault();
                ev.stopPropagation();
                setZoom(1.0);
            }
        };
        document.addEventListener("keydown", handleKey, { capture: true });
        onCleanup(() => document.removeEventListener("keydown", handleKey, { capture: true }));
    });

    const isFlipped = createMemo(() => model.selectedEntryAtom() != null);
    const goBack = () => model.setSelectedEntry(null);

    return (
        <div ref={rootRef} class="swarm-view" style={{ zoom: zoomFactor() }} tabIndex={-1}>
            <div class={`swarm-flip ${isFlipped() ? "flipped" : ""}`}>
                {/* Front face — tab bar + list */}
                <div class="swarm-face swarm-face--front">
                    <SwarmHeader model={model} />
                    <div class="swarm-content">
                        <Show when={model.tabAtom() === "active"}>
                            <ActiveList model={model} />
                        </Show>
                        <Show when={model.tabAtom() === "retired"}>
                            <RetiredList model={model} />
                        </Show>
                    </div>
                </div>

                {/* Back face — back arrow + entry detail */}
                <div class="swarm-face swarm-face--back">
                    <DetailHeader entry={model.selectedEntryAtom()} onBack={goBack} />
                    <div class="swarm-content">
                        <Show when={model.selectedEntryAtom()}>
                            {(entry) => <SwarmDetail model={model} entry={entry()} />}
                        </Show>
                    </div>
                </div>
            </div>
        </div>
    );
}

// ── Header (front face) ─────────────────────────────────────────────────

function SwarmHeader({ model }: { model: SwarmViewModel }): JSX.Element {
    return (
        <div class="swarm-header">
            <div class="swarm-tabs">
                <button
                    class={`swarm-tab ${model.tabAtom() === "active" ? "active" : ""}`}
                    onClick={() => model.setTab("active")}
                >
                    Active
                </button>
                <button
                    class={`swarm-tab ${model.tabAtom() === "retired" ? "active" : ""}`}
                    onClick={() => model.setTab("retired")}
                >
                    Retired
                </button>
            </div>
        </div>
    );
}

// ── Header (back face / detail) ─────────────────────────────────────────

function DetailHeader({
    entry,
    onBack,
}: {
    entry: SwarmEntry | null;
    onBack: () => void;
}): JSX.Element {
    const title = () => {
        if (!entry) return "";
        if (entry.kind === "subagent") {
            return entry.data.slug || entry.data.agent_id.substring(0, 7);
        }
        return "";
    };
    return (
        <div class="swarm-header swarm-header--detail">
            <button class="swarm-back" onClick={onBack} title="Back to list">
                {"←"}
            </button>
            <span class="swarm-detail-title">{title()}</span>
        </div>
    );
}

// ── Active tab body ─────────────────────────────────────────────────────

function ActiveList({ model }: { model: SwarmViewModel }): JSX.Element {
    const subagents = () =>
        model.subagentsAtom()
            .filter((s) => s.status === "active")
            .sort((a, b) => b.last_event_at - a.last_event_at);

    return (
        <div class="swarm-list">
            <Show
                when={subagents().length > 0}
                fallback={
                    <div class="swarm-empty-state">
                        <div class="swarm-empty-title">No active subagents</div>
                        <div class="swarm-empty-desc">
                            Subagents appear here while an agent runs parallel tasks via the Task tool.
                        </div>
                    </div>
                }
            >
                <For each={subagents()}>
                    {(sub) => (
                        <SubagentRow
                            subagent={sub}
                            onClick={() => model.setSelectedEntry({ kind: "subagent", data: sub })}
                        />
                    )}
                </For>
            </Show>
        </div>
    );
}

// ── Retired tab body ────────────────────────────────────────────────────

function RetiredList({ model }: { model: SwarmViewModel }): JSX.Element {
    const subagents = () =>
        model.subagentsAtom()
            .filter((s) => s.status === "completed")
            .sort((a, b) => b.last_event_at - a.last_event_at);

    return (
        <div class="swarm-list">
            <Show
                when={subagents().length > 0}
                fallback={
                    <div class="swarm-empty-state">
                        <div class="swarm-empty-title">No retired subagents</div>
                        <div class="swarm-empty-desc">
                            Completed subagents appear here when they exit.
                        </div>
                    </div>
                }
            >
                <For each={subagents()}>
                    {(sub) => (
                        <SubagentRow
                            subagent={sub}
                            onClick={() => model.setSelectedEntry({ kind: "subagent", data: sub })}
                        />
                    )}
                </For>
            </Show>
        </div>
    );
}

// ── List row ────────────────────────────────────────────────────────────

function SubagentRow({
    subagent,
    onClick,
}: {
    subagent: ActiveSubagent;
    onClick: () => void;
}): JSX.Element {
    const elapsed = () => {
        const ms = Date.now() - subagent.last_event_at;
        if (ms < 60000) return `${Math.floor(ms / 1000)}s ago`;
        if (ms < 3600000) return `${Math.floor(ms / 60000)}m ago`;
        return `${Math.floor(ms / 3600000)}h ago`;
    };
    const isActive = subagent.status === "active";
    return (
        <div class={`swarm-subagent-card ${isActive ? "active" : "completed"}`} onClick={onClick}>
            <span class="swarm-subagent-status">{isActive ? "\u{26A1}" : "✔"}</span>
            <div class="swarm-subagent-info">
                <span class="swarm-subagent-slug">
                    {subagent.slug || subagent.agent_id.substring(0, 7)}
                </span>
                <span class="swarm-subagent-meta">
                    {subagent.parent_agent} {"›"} {subagent.agent_id.substring(0, 7)}
                </span>
            </div>
            <div class="swarm-subagent-stats">
                <span class="swarm-subagent-events">{subagent.event_count} events</span>
                <span class="swarm-subagent-time">{elapsed()}</span>
            </div>
        </div>
    );
}

// ── Detail face ─────────────────────────────────────────────────────────

function SwarmDetail({
    model,
    entry,
}: {
    model: SwarmViewModel;
    entry: SwarmEntry;
}): JSX.Element {
    if (entry.kind !== "subagent") return null as unknown as JSX.Element;
    const sub = entry.data;
    const elapsed = () => {
        const ms = Date.now() - sub.last_event_at;
        if (ms < 60000) return `${Math.floor(ms / 1000)}s ago`;
        if (ms < 3600000) return `${Math.floor(ms / 60000)}m ago`;
        return `${Math.floor(ms / 3600000)}h ago`;
    };
    const handleOpen = () => {
        if (isSubagentPaneOpen(sub.agent_id)) return;
        void openSubagentPane({
            subagentId: sub.agent_id,
            slug: sub.slug,
            parentAgent: sub.parent_agent,
            parentBlockId: model.blockId,
            sessionId: sub.session_id,
        });
    };
    return (
        <div class="swarm-detail">
            <dl class="swarm-detail-grid">
                <dt>Status</dt>
                <dd>{sub.status}</dd>
                <dt>Parent agent</dt>
                <dd>{sub.parent_agent}</dd>
                <dt>Subagent id</dt>
                <dd class="mono">{sub.agent_id}</dd>
                <dt>Session id</dt>
                <dd class="mono">{sub.session_id || "—"}</dd>
                <dt>Model</dt>
                <dd>{sub.model || "—"}</dd>
                <dt>Events</dt>
                <dd>{sub.event_count}</dd>
                <dt>Last event</dt>
                <dd>{elapsed()}</dd>
            </dl>
            <div class="swarm-detail-actions">
                <button class="swarm-detail-action" onClick={handleOpen}>
                    Open subagent pane
                </button>
            </div>
        </div>
    );
}
