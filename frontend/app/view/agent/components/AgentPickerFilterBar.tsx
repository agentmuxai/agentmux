// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPickerFilterBar — a static, always-visible text filter atop the
 * AgentPicker, narrowing `MyAgentsList` by name as the user types.
 *
 * Distinct from `AgentSearchBar.tsx` in this same directory, which is an
 * unrelated feature: the in-session transcript search overlay (Ctrl+F),
 * toggled visible/hidden and searching loaded conversation nodes within
 * one agent's pane. This component is always rendered (no show/hide
 * toggle) and searches agent names, not message content.
 *
 * Purely presentational — no data fetching of its own. `AgentPicker.tsx`
 * owns the query signal and threads it into `MyAgentsList` as `nameFilter`.
 * See docs/specs/SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md.
 */

import { Show, type Accessor, type JSX } from "solid-js";

export interface AgentPickerFilterBarProps {
    value: Accessor<string>;
    onInput: (query: string) => void;
    onClear: () => void;
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
        </div>
    );
};

AgentPickerFilterBar.displayName = "AgentPickerFilterBar";
