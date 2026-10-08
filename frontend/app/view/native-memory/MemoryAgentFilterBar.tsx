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
 * `native-memory-manager.tsx` owns the signals and does the actual filtering.
 */

import { type Accessor, type JSX } from "solid-js";
import { FilterInput } from "@/app/element/ui";

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
    return (
        <div class="memory-agent-filter-bar" data-testid="memory-agent-filter-bar">
            {/* Icon + input + clear button share one flex child (ReAgent P2,
                PR #2929): flex-basis:100% on the input ALONE (the earlier
                shape of this markup) let the icon claim an empty first line
                by itself before the input's 100% hypothetical size forced
                it to wrap to a second line — two items can't both claim the
                full row. Grouping them means the narrow-pane flex-basis:100%
                rule (see the .scss) applies to the trio atomically. */}
            <span class="memory-agent-filter-search">
                <FilterInput
                    bare
                    value={props.value()}
                    onInput={props.onInput}
                    onClear={props.onClear}
                    placeholder="Filter agents..."
                    iconClass="memory-agent-filter-icon"
                    inputClass="memory-agent-filter-input"
                    clearClass="memory-agent-filter-clear"
                    testId="memory-agent-filter"
                />
            </span>
            <label class="memory-agent-filter-toggle" data-testid="memory-agent-filter-toggle" title="Has memories">
                <input
                    type="checkbox"
                    checked={props.onlyWithMemories()}
                    onChange={(e) => props.onOnlyWithMemoriesChange(e.currentTarget.checked)}
                    // The full text label hides at narrow pane widths in favor
                    // of the abbreviated span below (Codex P2×2, PR #2929 —
                    // hiding it with no replacement left a naked checkbox with
                    // no visible explanation for sighted, non-screen-reader
                    // users; the `title` attribute above adds a hover tooltip
                    // at any width too). This aria-label keeps the checkbox
                    // labeled for a11y independent of either span's own
                    // visibility, since `display: none` on a span also
                    // removes it from the accessibility tree, not just the
                    // visual layout.
                    aria-label="Has memories"
                />
                <span class="memory-agent-filter-toggle-label-full">Has memories</span>
                <span class="memory-agent-filter-toggle-label-compact" aria-hidden="true">Mem</span>
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
