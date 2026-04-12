// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSearchBar — in-session search overlay for agent panes.
 *
 * Appears as a sticky banner at the top of the pane when Ctrl+F is pressed.
 * Performs case-insensitive substring search over the currently-loaded document
 * nodes (the in-memory slice, not the full persisted history — searching the
 * full history would require a backend blockfile:search RPC, which is out of
 * scope for this PR).
 *
 * Keyboard handling:
 *   Enter        → next match
 *   Shift+Enter  → previous match
 *   Escape       → close
 */

import { Show, type Accessor, type JSX } from "solid-js";

export interface AgentSearchBarProps {
    visible: Accessor<boolean>;
    onSearch: (query: string) => void;
    onNext: () => void;
    onPrev: () => void;
    onClose: () => void;
    /** 0-based index of the current match. -1 if no matches. */
    matchIndex: Accessor<number>;
    matchCount: Accessor<number>;
}

export const AgentSearchBar = (props: AgentSearchBarProps): JSX.Element => {
    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.preventDefault();
            props.onClose();
        } else if (e.key === "Enter") {
            e.preventDefault();
            if (e.shiftKey) props.onPrev();
            else props.onNext();
        }
    };

    const counterText = () => {
        const count = props.matchCount();
        if (count === 0) return "0 matches";
        return `${props.matchIndex() + 1} of ${count}`;
    };

    return (
        <Show when={props.visible()}>
            <div class="agent-search-bar">
                <input
                    ref={(el) => {
                        // Auto-focus when the bar appears. The setTimeout defers
                        // the focus call past the current render frame so the
                        // element is guaranteed to be mounted and visible.
                        setTimeout(() => el?.focus(), 0);
                    }}
                    class="agent-search-input"
                    type="text"
                    placeholder="Search messages..."
                    onInput={(e) => {
                        props.onSearch(e.currentTarget.value);
                    }}
                    onKeyDown={handleKeyDown}
                />
                <span class="agent-search-counter">{counterText()}</span>
                <button
                    class="agent-search-btn"
                    onClick={props.onPrev}
                    title="Previous match (Shift+Enter)"
                >
                    &#x25B2;
                </button>
                <button
                    class="agent-search-btn"
                    onClick={props.onNext}
                    title="Next match (Enter)"
                >
                    &#x25BC;
                </button>
                <button
                    class="agent-search-btn agent-search-btn--close"
                    onClick={props.onClose}
                    title="Close search (Esc)"
                >
                    &times;
                </button>
            </div>
        </Show>
    );
};

AgentSearchBar.displayName = "AgentSearchBar";
