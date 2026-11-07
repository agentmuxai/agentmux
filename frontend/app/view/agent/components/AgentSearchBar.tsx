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
import { IconButton, TextInput } from "@/app/element/ui";

export interface AgentSearchBarProps {
    visible: Accessor<boolean>;
    onSearch: (query: string) => void;
    onNext: () => void;
    onPrev: () => void;
    onClose: () => void;
    /** 0-based index of the current match. -1 if no matches. */
    matchIndex: Accessor<number>;
    matchCount: Accessor<number>;
    /** A pass is taking long enough to say so. */
    searching?: Accessor<boolean>;
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
        if (props.searching?.()) return "Searching…";
        const count = props.matchCount();
        if (count === 0) return "No matches";
        // Matches were found while none is current (rows arrived after a search that had none): count them.
        if (props.matchIndex() < 0) return count === 1 ? "1 match" : `${count} matches`;
        return `${props.matchIndex() + 1} of ${count}`;
    };

    return (
        <Show when={props.visible()}>
            <div class="agent-search-bar">
                <TextInput
                    variant="plain"
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
                <IconButton icon="caret-up" label="Previous match (Shift+Enter)" density="compact" onClick={() => props.onPrev()} />
                <IconButton icon="caret-down" label="Next match (Enter)" density="compact" onClick={() => props.onNext()} />
                <IconButton icon="xmark" label="Close search (Esc)" density="compact" onClick={() => props.onClose()} />
            </div>
        </Show>
    );
};

AgentSearchBar.displayName = "AgentSearchBar";
