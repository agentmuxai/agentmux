// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";

export function ForgeHistoryPanel(props: { model: ForgeViewModel; agentId: string }): JSX.Element {
    const entries = props.model.historyAtom;
    const loading = props.model.historyLoadingAtom;
    const [searchQuery, setSearchQuery] = createSignal("");

    const handleSearch = async () => {
        if (searchQuery().trim()) {
            await props.model.searchHistory(props.agentId, searchQuery().trim());
        } else {
            await props.model.loadHistory(props.agentId);
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            handleSearch();
        }
    };

    // Group entries by session_date
    const groupedEntries = () => {
        return entries().reduce<Record<string, ForgeHistory[]>>((acc, entry) => {
            const date = entry.session_date;
            if (!acc[date]) acc[date] = [];
            acc[date].push(entry);
            return acc;
        }, {});
    };
    const sortedDates = () => Object.keys(groupedEntries()).sort().reverse();

    return (
        <div class="forge-history-panel">
            <div class="forge-history-search">
                <input
                    class="forge-form-input"
                    value={searchQuery()}
                    onInput={(e) => setSearchQuery(e.currentTarget.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Search history..."
                />
                <button class="forge-card-btn" onClick={handleSearch}>
                    Search
                </button>
            </div>
            <div class="forge-history-list">
                <Show when={!loading()} fallback={
                    <div class="forge-content-loading">Loading history...</div>
                }>
                    <Show when={entries().length > 0} fallback={
                        <div class="forge-content-empty">No history entries</div>
                    }>
                        <For each={sortedDates()}>{(date) =>
                            <div class="forge-history-group">
                                <div class="forge-history-date">{date}</div>
                                <For each={groupedEntries()[date]}>{(entry) =>
                                    <div class="forge-history-entry">
                                        <span class="forge-history-time">
                                            {new Date(entry.timestamp).toLocaleTimeString()}
                                        </span>
                                        <span class="forge-history-text">{entry.entry}</span>
                                    </div>
                                }</For>
                            </div>
                        }</For>
                    </Show>
                </Show>
            </div>
        </div>
    );
}
