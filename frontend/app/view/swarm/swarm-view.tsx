// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, Show, type JSX } from "solid-js";
import type { SwarmViewModel, ActiveSubagent, HistorySessionMeta, HistoryMessage } from "./swarm-model";
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
                        class={`swarm-tab ${model.tabAtom() === "history" ? "active" : ""}`}
                        onClick={() => {
                            model.setTab("history");
                            model.loadHistory();
                        }}
                    >
                        History
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
                <Show when={model.tabAtom() === "history"}>
                    <SwarmHistory model={model} />
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

// ── History Tab ─────────────────────────────────────────────────────────

function SwarmHistory({ model }: { model: SwarmViewModel }): JSX.Element {
    const formatDate = (ms: number): string => {
        if (!ms) return "";
        const d = new Date(ms);
        const now = new Date();
        const diffMs = now.getTime() - d.getTime();
        const diffDays = Math.floor(diffMs / 86400000);
        if (diffDays === 0) return `Today ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
        if (diffDays === 1) return `Yesterday ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
        if (diffDays < 7) return `${diffDays}d ago`;
        return d.toLocaleDateString([], { month: "short", day: "numeric" });
    };

    const formatSize = (bytes: number): string => {
        if (bytes < 1024) return `${bytes}B`;
        if (bytes < 1048576) return `${(bytes / 1024).toFixed(0)}KB`;
        return `${(bytes / 1048576).toFixed(1)}MB`;
    };

    const truncateMsg = (msg: string, len = 80): string => {
        if (!msg) return "(no message)";
        const clean = msg.replace(/\n/g, " ").trim();
        return clean.length > len ? clean.substring(0, len) + "..." : clean;
    };

    const handleSessionClick = (session: HistorySessionMeta) => {
        model.loadSession(session.session_id);
    };

    const handleLoadMore = () => {
        model.loadHistory(model.historySessionsAtom().length);
    };

    return (
        <div class="swarm-history">
            <Show when={model.selectedSessionAtom() != null}>
                <SwarmConversationViewer model={model} />
            </Show>
            <Show when={model.selectedSessionAtom() == null}>
                {/* Header bar */}
                <div class="swarm-history-toolbar">
                    <span class="swarm-history-count">
                        {model.historyTotalAtom()} sessions
                    </span>
                    <button
                        class="swarm-history-refresh"
                        onClick={() => model.refreshHistory()}
                        disabled={model.historyLoadingAtom()}
                    >
                        {model.historyLoadingAtom() ? "Scanning..." : "Refresh"}
                    </button>
                </div>

                {/* Session list */}
                <Show when={model.historySessionsAtom().length > 0}>
                    <div class="swarm-history-list">
                        <For each={model.historySessionsAtom()}>
                            {(session) => (
                                <div
                                    class="swarm-history-item"
                                    onClick={() => handleSessionClick(session)}
                                >
                                    <div class="swarm-history-item-header">
                                        <span class="swarm-history-item-slug">
                                            {session.slug || session.session_id.substring(0, 12)}
                                        </span>
                                        <span class="swarm-history-item-date">
                                            {formatDate(session.modified_at)}
                                        </span>
                                    </div>
                                    <div class="swarm-history-item-preview">
                                        {truncateMsg(session.first_user_message)}
                                    </div>
                                    <div class="swarm-history-item-meta">
                                        <span>{session.message_count} msgs</span>
                                        <span>{formatSize(session.file_size_bytes)}</span>
                                        <Show when={session.model !== "unknown"}>
                                            <span>{session.model}</span>
                                        </Show>
                                        <Show when={session.subagent_count > 0}>
                                            <span>{session.subagent_count} subagents</span>
                                        </Show>
                                    </div>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>

                <Show when={model.historyHasMoreAtom()}>
                    <div class="swarm-history-load-more">
                        <button
                            class="swarm-history-load-more-btn"
                            onClick={handleLoadMore}
                            disabled={model.historyLoadingAtom()}
                        >
                            Load More
                        </button>
                    </div>
                </Show>

                <Show when={!model.historyLoadingAtom() && model.historySessionsAtom().length === 0}>
                    <div class="swarm-empty-state">
                        <div class="swarm-empty-icon">{"\u{1F4DC}"}</div>
                        <div class="swarm-empty-title">No History Found</div>
                        <div class="swarm-empty-desc">
                            Click Refresh to scan for Claude Code session files on disk.
                        </div>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

// ── Conversation Viewer ─────────────────────────────────────────────────

function SwarmConversationViewer({ model }: { model: SwarmViewModel }): JSX.Element {
    const session = () => model.selectedSessionAtom();
    const meta = () => session()?.meta;

    const formatTimestamp = (ms: number): string => {
        if (!ms) return "";
        return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
    };

    return (
        <div class="swarm-conversation">
            <div class="swarm-conversation-header">
                <button class="swarm-conversation-back" onClick={() => model.closeSession()}>
                    {"\u2190"} Back
                </button>
                <div class="swarm-conversation-title">
                    <span class="swarm-conversation-slug">
                        {meta()?.slug || meta()?.session_id?.substring(0, 12)}
                    </span>
                    <span class="swarm-conversation-meta">
                        {meta()?.message_count} messages
                        {meta()?.model !== "unknown" ? ` \u00B7 ${meta()?.model}` : ""}
                    </span>
                </div>
            </div>

            <Show when={model.sessionLoadingAtom()}>
                <div class="swarm-empty">Loading conversation...</div>
            </Show>

            <Show when={!model.sessionLoadingAtom() && session()}>
                <div class="swarm-conversation-messages">
                    <For each={session()!.messages}>
                        {(msg: HistoryMessage) => (
                            <div class={`swarm-message swarm-message-${msg.role}`}>
                                <div class="swarm-message-header">
                                    <span class="swarm-message-role">
                                        {msg.role === "user" ? "User" : "Assistant"}
                                    </span>
                                    <span class="swarm-message-time">
                                        {formatTimestamp(msg.timestamp)}
                                    </span>
                                </div>
                                <Show when={msg.content}>
                                    <div class="swarm-message-content">
                                        {msg.content}
                                    </div>
                                </Show>
                                <Show when={msg.tool_uses.length > 0}>
                                    <div class="swarm-message-tools">
                                        <For each={msg.tool_uses}>
                                            {(tool) => (
                                                <div class="swarm-tool-use">
                                                    <span class="swarm-tool-name">{tool.name}</span>
                                                    <Show when={tool.argument_summary}>
                                                        <span class="swarm-tool-args">
                                                            {tool.argument_summary}
                                                        </span>
                                                    </Show>
                                                </div>
                                            )}
                                        </For>
                                    </div>
                                </Show>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
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
