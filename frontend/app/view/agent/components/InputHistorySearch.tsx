// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ctrl+R in the composer: search what was typed to this agent, as a shell's
 * reverse search does (docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md §4.6).
 * Type to filter, Up/Down (or Ctrl+R again) to move, Enter to put the entry in
 * the composer, Escape to go back to it. Shift+Delete forgets the selected
 * entry; "Clear history" forgets them all, after asking.
 */

import type { JSX } from "solid-js";
import { EntryList, SearchField, SearchFooter } from "./input-history-search-parts";
import { createInputHistorySearch, type InputHistorySearchProps } from "./input-history-search-state";

export function InputHistorySearch(props: InputHistorySearchProps): JSX.Element {
    const s = createInputHistorySearch(props);

    return (
        <div class="input-history-search" role="dialog" aria-label="Search what you typed to this agent">
            <SearchField s={s} onDone={props.onDone} />
            <EntryList s={s} onDone={props.onDone} />
            <SearchFooter s={s} />
        </div>
    );
}
