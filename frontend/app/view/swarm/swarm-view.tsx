// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, Show, type JSX } from "solid-js";
import type { SwarmViewModel, ActiveSubagent } from "./swarm-model";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import "./swarm-view.scss";

export function SwarmView(props: ViewComponentProps<SwarmViewModel>): JSX.Element {
    const model = props.model;

    return (
        <div class="swarm-view">
            <div class="swarm-header">
                <span class="swarm-header-icon">{"\u{1F310}"}</span>
                <span class="swarm-header-title">Swarm</span>
                <div class="swarm-tabs">
                    <button
                        class={`swarm-tab ${model.tabAtom() === "overview" ? "active" : ""}`}
                        onClick={() => model.setTab("overview")}
                    >
                        Overview
                    </button>
                    <button
                        class={`swarm-tab ${model.tabAtom() === "search" ? "active" : ""}`}
                        onClick={() => model.setTab("search")}
                    >
                        Search
                    </button>
                </div>
            </div>

            <div class="swarm-content">
                <Show when={model.tabAtom() === "overview"}>
                    <SwarmOverview model={model} />
                </Show>
                <Show when={model.tabAtom() === "search"}>
                    <SwarmSearch model={model} />
                </Show>
            </div>
        </div>
    );
}

// ── Overview Tab ────────────────────────────────────────────────────────

function SwarmOverview({ model }: { model: SwarmViewModel }): JSX.Element {
    const activeSubagents = () => model.subagentsAtom().filter((s) => s.status === "active");
    const completedSubagents = () => model.subagentsAtom().filter((s) => s.status === "completed");

    const handleSubagentClick = (sub: ActiveSubagent) => {
        if (isSubagentPaneOpen(sub.agent_id)) return;
        openSubagentPane({
            subagentId: sub.agent_id,
            slug: sub.slug,
            parentAgent: sub.parent_agent,
            parentBlockId: model.blockId,
            sessionId: sub.session_id,
        });
    };

    return (
        <div class="swarm-overview">
            {/* Active Subagents */}
            <div class="swarm-section">
                <div class="swarm-section-header">
                    <span class="swarm-section-title">
                        Active Subagents ({activeSubagents().length})
                    </span>
                </div>
                <Show
                    when={activeSubagents().length > 0}
                    fallback={
                        <div class="swarm-empty">No active subagents</div>
                    }
                >
                    <div class="swarm-subagent-list">
                        <For each={activeSubagents()}>
                            {(sub) => (
                                <SubagentCard
                                    subagent={sub}
                                    onClick={() => handleSubagentClick(sub)}
                                />
                            )}
                        </For>
                    </div>
                </Show>
            </div>

            {/* Recently Completed */}
            <Show when={completedSubagents().length > 0}>
                <div class="swarm-section">
                    <div class="swarm-section-header">
                        <span class="swarm-section-title">
                            Recently Completed ({completedSubagents().length})
                        </span>
                    </div>
                    <div class="swarm-subagent-list">
                        <For each={completedSubagents()}>
                            {(sub) => (
                                <SubagentCard
                                    subagent={sub}
                                    onClick={() => handleSubagentClick(sub)}
                                />
                            )}
                        </For>
                    </div>
                </div>
            </Show>

            <Show when={model.subagentsAtom().length === 0 && !model.loadingAtom()}>
                <div class="swarm-empty-state">
                    <div class="swarm-empty-icon">{"\u{1F310}"}</div>
                    <div class="swarm-empty-title">No Swarm Activity</div>
                    <div class="swarm-empty-desc">
                        Subagents will appear here when an agent spawns
                        parallel tasks via the Task tool.
                    </div>
                </div>
            </Show>
        </div>
    );
}

// ── Subagent Card ───────────────────────────────────────────────────────

function SubagentCard({
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
        <div
            class={`swarm-subagent-card ${isActive ? "active" : "completed"}`}
            onClick={onClick}
        >
            <span class="swarm-subagent-status">
                {isActive ? "\u{26A1}" : "\u2714"}
            </span>
            <div class="swarm-subagent-info">
                <span class="swarm-subagent-slug">
                    {subagent.slug || subagent.agent_id.substring(0, 7)}
                </span>
                <span class="swarm-subagent-meta">
                    {subagent.parent_agent} {"\u203A"}{" "}
                    {subagent.agent_id.substring(0, 7)}
                </span>
            </div>
            <div class="swarm-subagent-stats">
                <span class="swarm-subagent-events">
                    {subagent.event_count} events
                </span>
                <span class="swarm-subagent-time">{elapsed()}</span>
            </div>
        </div>
    );
}

// ── Search Tab ──────────────────────────────────────────────────────────

function SwarmSearch({ model }: { model: SwarmViewModel }): JSX.Element {
    let inputRef!: HTMLInputElement;
    const [localQuery, setLocalQuery] = createSignal(model.searchQueryAtom());

    const handleSearch = () => {
        const q = localQuery().trim();
        model.setSearchQuery(q);
        model.search(q);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            handleSearch();
        }
    };

    return (
        <div class="swarm-search">
            <div class="swarm-search-bar">
                <input
                    ref={inputRef}
                    class="swarm-search-input"
                    type="text"
                    placeholder="Search subagent conversations..."
                    value={localQuery()}
                    onInput={(e) => setLocalQuery(e.currentTarget.value)}
                    onKeyDown={handleKeyDown}
                />
                <button
                    class="swarm-search-btn"
                    onClick={handleSearch}
                    disabled={model.searchingAtom()}
                >
                    {model.searchingAtom() ? "..." : "Search"}
                </button>
            </div>

            <Show when={model.searchResultsAtom().length > 0}>
                <div class="swarm-search-results">
                    <For each={model.searchResultsAtom()}>
                        {(result) => (
                            <div class="swarm-search-result">
                                <div class="swarm-search-result-header">
                                    <span class="swarm-search-result-agent">
                                        {result.agent_id.substring(0, 7)}
                                    </span>
                                    <span class="swarm-search-result-type">
                                        {result.event_type}
                                    </span>
                                    <span class="swarm-search-result-time">
                                        {new Date(
                                            result.timestamp
                                        ).toLocaleString()}
                                    </span>
                                </div>
                                <div class="swarm-search-result-preview">
                                    {result.preview}
                                </div>
                            </div>
                        )}
                    </For>
                </div>
            </Show>

            <Show
                when={
                    model.searchQueryAtom() &&
                    !model.searchingAtom() &&
                    model.searchResultsAtom().length === 0
                }
            >
                <div class="swarm-empty">No results found</div>
            </Show>
        </div>
    );
}
