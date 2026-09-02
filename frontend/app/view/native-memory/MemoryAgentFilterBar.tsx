// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryAgentFilterBar — a text filter + "Has memories" toggle + sort
 * control atop the Personal Memory grid, narrowing/ordering which
 * `MemoryAgentCard`s are shown. Mirrors the interaction pattern of
 * `AgentPickerFilterBar.tsx` (Agent pane "My Agents") — magnifying-glass
 * icon, clear button, Escape-to-clear, sort `<select>` pinned right — but is
 * NOT that component reused: `AgentSortOption` ("recent"/"name"/"type") is
 * launch-oriented and reads session-launch data this grid doesn't have.
 * `MemoryAgentSortOption` below is its own, memory-grid-specific enum. See
 * docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md.
 *
 * Purely presentational — no data fetching/filtering/sorting of its own.
 * `NativeMemoryManager.tsx` owns the signals and does the actual filtering.
 */

import { Show, type Accessor, type JSX } from "solid-js";

export type MemoryAgentSortOption = "name" | "count" | "provider";

export const DEFAULT_MEMORY_SORT: MemoryAgentSortOption = "name";

export interface MemoryAgentFilterBarProps {
    value: Accessor<string>;
    onInput: (query: string) => void;
    onClear: () => void;
    onlyWithMemories: Accessor<boolean>;
    onOnlyWithMemoriesChange: (value: boolean) => void;
    sortBy: Accessor<MemoryAgentSortOption>;
    onSortChange: (sort: MemoryAgentSortOption) => void;
}

export const MemoryAgentFilterBar = (props: MemoryAgentFilterBarProps): JSX.Element => {
    const handleKeyDown = (e: KeyboardEvent) => {
        // Same convention as AgentPickerFilterBar: clears the field without
        // hiding the bar (this bar has no hidden state to return to).
        if (e.key === "Escape" && props.value()) {
            e.preventDefault();
            props.onClear();
        }
    };

    return (
        <div class="memory-agent-filter-bar" data-testid="memory-agent-filter-bar">
            <i class="fa-solid fa-magnifying-glass memory-agent-filter-icon" aria-hidden="true" />
            <input
                type="text"
                class="memory-agent-filter-input"
                placeholder="Filter agents..."
                value={props.value()}
                data-testid="memory-agent-filter-input"
                onInput={(e) => props.onInput(e.currentTarget.value)}
                onKeyDown={handleKeyDown}
            />
            <Show when={props.value()}>
                <button
                    type="button"
                    class="memory-agent-filter-clear"
                    onClick={() => props.onClear()}
                    aria-label="Clear filter"
                    data-testid="memory-agent-filter-clear"
                >
                    &times;
                </button>
            </Show>
            <label class="memory-agent-filter-toggle" data-testid="memory-agent-filter-toggle">
                <input
                    type="checkbox"
                    checked={props.onlyWithMemories()}
                    onChange={(e) => props.onOnlyWithMemoriesChange(e.currentTarget.checked)}
                />
                <span>Has memories</span>
            </label>
            <label class="memory-agent-sort" data-testid="memory-agent-sort">
                <span class="memory-agent-sort-label">Sort</span>
                <select
                    class="memory-agent-sort-select"
                    aria-label="Sort agents by"
                    data-testid="memory-agent-sort-select"
                    value={props.sortBy()}
                    onChange={(e) => props.onSortChange(e.currentTarget.value as MemoryAgentSortOption)}
                >
                    <option value="name">Name (A–Z)</option>
                    <option value="count">Most files</option>
                    <option value="provider">Provider</option>
                </select>
            </label>
        </div>
    );
};

MemoryAgentFilterBar.displayName = "MemoryAgentFilterBar";
