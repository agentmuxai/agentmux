// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPickerFilterBar — a static, always-visible text filter atop the
 * AgentPicker, narrowing `MyAgentsList` by name as the user types, plus a
 * sort control (name / recently launched / type) at the far right —
 * SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md follow-up, see
 * docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §3.
 *
 * Distinct from `AgentSearchBar.tsx` in this same directory, which is an
 * unrelated feature: the in-session transcript search overlay (Ctrl+F),
 * toggled visible/hidden and searching loaded conversation nodes within
 * one agent's pane. This component is always rendered (no show/hide
 * toggle) and searches agent names, not message content.
 *
 * Purely presentational — no data fetching/sorting of its own.
 * `AgentPicker.tsx` owns both signals and threads them into `MyAgentsList`
 * as `nameFilter`/`sortBy`. See docs/specs/SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md.
 */

import { Show, type Accessor, type JSX } from "solid-js";

/** "Recently launched" (most recent `started_at` first) is the default —
 *  it matches the backend's own existing sort intent (see
 *  `listrecentsessions`'s row ordering before the picker ever added a
 *  user-facing sort control at all), so leaving the control untouched
 *  reproduces today's behavior exactly. */
export type AgentSortOption = "recent" | "name" | "type";

export const DEFAULT_AGENT_SORT: AgentSortOption = "recent";

export interface AgentPickerFilterBarProps {
    value: Accessor<string>;
    onInput: (query: string) => void;
    onClear: () => void;
    sortBy: Accessor<AgentSortOption>;
    onSortChange: (sort: AgentSortOption) => void;
}

export const AgentPickerFilterBar = (props: AgentPickerFilterBarProps): JSX.Element => {
    const handleKeyDown = (e: KeyboardEvent) => {
        // Clears the field, same as the AgentSearchBar convention — but
        // doesn't hide the bar, since (unlike that Ctrl+F overlay) this
        // bar has no hidden state to return to.
        if (e.key === "Escape" && props.value()) {
            e.preventDefault();
            props.onClear();
        }
    };

    return (
        <div class="agent-picker-filter-bar" data-testid="agent-picker-filter-bar">
            <i class="fa-solid fa-magnifying-glass agent-picker-filter-icon" aria-hidden="true" />
            <input
                type="text"
                class="agent-picker-filter-input"
                placeholder="Filter agents..."
                value={props.value()}
                data-testid="agent-picker-filter-input"
                onInput={(e) => props.onInput(e.currentTarget.value)}
                onKeyDown={handleKeyDown}
            />
            <Show when={props.value()}>
                <button
                    type="button"
                    class="agent-picker-filter-clear"
                    onClick={() => props.onClear()}
                    aria-label="Clear filter"
                    data-testid="agent-picker-filter-clear"
                >
                    &times;
                </button>
            </Show>
            <label class="agent-picker-sort" data-testid="agent-picker-sort">
                <span class="agent-picker-sort-label">Sort</span>
                <select
                    class="agent-picker-sort-select"
                    aria-label="Sort My Agents by"
                    data-testid="agent-picker-sort-select"
                    value={props.sortBy()}
                    onChange={(e) => props.onSortChange(e.currentTarget.value as AgentSortOption)}
                >
                    <option value="recent">Recently launched</option>
                    <option value="name">Name (A–Z)</option>
                    <option value="type">Type</option>
                </select>
            </label>
        </div>
    );
};

AgentPickerFilterBar.displayName = "AgentPickerFilterBar";
