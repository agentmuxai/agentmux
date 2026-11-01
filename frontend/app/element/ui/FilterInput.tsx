// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { Show, type JSX } from "solid-js";

export interface FilterInputProps {
    value: string;
    onInput: (query: string) => void;
    /** Defaults to `onInput("")`. */
    onClear?: () => void;
    placeholder: string;
    /** Defaults to the placeholder. */
    ariaLabel?: string;
    /** The pane's typing target (`data-pane-focus`, focusManager). Default true. */
    paneFocus?: boolean;
    /** The input element, for a caller that moves the focus to it. */
    inputRef?: (el: HTMLInputElement) => void;
    /** Runs first on every key. A key it handles (`preventDefault`) skips the Escape-clears behaviour. */
    onKeyDown?: (e: KeyboardEvent) => void;
    class?: string;
    /** `${testId}-input` and `${testId}-clear`. */
    testId?: string;
}

/**
 * The magnifier of a search or filter field: a square cell as tall as the field, the glyph centred in it,
 * so the room left of the glyph, right of it (up to the text), above and below it is the same. A field
 * that is not a FilterInput (the command palette) puts this at the start of its row.
 */
export function SearchIcon(props: { class?: string }): JSX.Element {
    return <i class={clsx("fa-solid fa-magnifying-glass ui-search-icon", props.class)} aria-hidden="true" />;
}

/**
 * The one "type to narrow this list" box (SPEC_HELP_PANE_FILTER_2026_10_08.md
 * §4): magnifier, input, a clear button while there is text, and Escape clears.
 * Matching is the caller's, usually `matchesEveryWord` (util/fuzzysearch.ts).
 */
export function FilterInput(props: FilterInputProps): JSX.Element {
    const clear = () => (props.onClear ? props.onClear() : props.onInput(""));
    return (
        <span class={clsx("ui-filter", props.class)}>
            <SearchIcon />
            <input
                ref={(el) => props.inputRef?.(el)}
                type="text"
                class="ui-filter-input"
                data-pane-focus={props.paneFocus === false ? undefined : ""}
                placeholder={props.placeholder}
                aria-label={props.ariaLabel ?? props.placeholder}
                value={props.value}
                data-testid={props.testId ? `${props.testId}-input` : undefined}
                onInput={(e) => props.onInput(e.currentTarget.value)}
                onKeyDown={(e) => {
                    props.onKeyDown?.(e);
                    if (e.defaultPrevented) return;
                    if (e.key === "Escape" && props.value) {
                        e.preventDefault();
                        clear();
                    }
                }}
            />
            <Show when={props.value}>
                <button
                    type="button"
                    class="ui-filter-clear"
                    onClick={() => clear()}
                    aria-label="Clear filter"
                    data-testid={props.testId ? `${props.testId}-clear` : undefined}
                >
                    <i class="fa-solid fa-xmark" aria-hidden="true" />
                </button>
            </Show>
        </span>
    );
}
