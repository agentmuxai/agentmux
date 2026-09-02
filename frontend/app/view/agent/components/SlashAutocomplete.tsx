// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SlashAutocomplete — dropdown that appears above the composer textarea
 * when the user types `/` followed by a partial command name.
 *
 * Step 3 of docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * Pure presentation: receives the completion list + current selection
 * index from AgentFooter, renders rows. AgentFooter owns the keyboard
 * handling (so Tab/Enter inside the textarea reach the autocomplete
 * before reaching send).
 */

import { type JSX } from "solid-js";
import { For } from "solid-js/web";
import type { SlashCommand } from "../commands/types";

interface SlashAutocompleteProps {
    completions: SlashCommand[];
    selectedIndex: number;
    onHover: (index: number) => void;
    onSelect: (cmd: SlashCommand) => void;
}

export function SlashAutocomplete(props: SlashAutocompleteProps): JSX.Element {
    return (
        <div class="slash-autocomplete" role="listbox" aria-label="Slash command suggestions">
            <For each={props.completions}>
                {(cmd, idx) => (
                    <div
                        class="slash-autocomplete__row"
                        classList={{
                            "slash-autocomplete__row--selected": idx() === props.selectedIndex,
                        }}
                        role="option"
                        aria-selected={idx() === props.selectedIndex}
                        onMouseEnter={() => props.onHover(idx())}
                        onMouseDown={(e) => {
                            // mousedown beats blur so the textarea keeps focus
                            e.preventDefault();
                            props.onSelect(cmd);
                        }}
                    >
                        <span class="slash-autocomplete__name">/{cmd.name}</span>
                        <span class="slash-autocomplete__description">{cmd.description}</span>
                    </div>
                )}
            </For>
            <div class="slash-autocomplete__hint">
                <span>↑↓</span>
                <span>Tab/↵ accept</span>
                <span>Esc dismiss</span>
            </div>
        </div>
    );
}
