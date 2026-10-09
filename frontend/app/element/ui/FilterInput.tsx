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
    /** Just the parts, for a bar that lays them out itself; otherwise they come
     *  in a bordered `.ui-filter` box. */
    bare?: boolean;
    class?: string;
    iconClass?: string;
    inputClass?: string;
    clearClass?: string;
    /** `${testId}-input` and `${testId}-clear`. */
    testId?: string;
}

/**
 * The one "type to narrow this list" box (SPEC_HELP_PANE_FILTER_2026_10_08.md
 * §4): magnifier, input, a clear button while there is text, and Escape clears.
 * Matching is the caller's, usually `matchesEveryWord` (util/fuzzysearch.ts).
 */
export function FilterInput(props: FilterInputProps): JSX.Element {
    const clear = () => (props.onClear ? props.onClear() : props.onInput(""));
    const parts = (
        <>
            <i class={clsx("fa-solid fa-magnifying-glass ui-filter-icon", props.iconClass)} aria-hidden="true" />
            <input
                type="text"
                class={clsx("ui-filter-input", props.inputClass)}
                data-pane-focus={props.paneFocus === false ? undefined : ""}
                placeholder={props.placeholder}
                aria-label={props.ariaLabel ?? props.placeholder}
                value={props.value}
                data-testid={props.testId ? `${props.testId}-input` : undefined}
                onInput={(e) => props.onInput(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Escape" && props.value) {
                        e.preventDefault();
                        clear();
                    }
                }}
            />
            <Show when={props.value}>
                <button
                    type="button"
                    class={clsx("ui-filter-clear", props.clearClass)}
                    onClick={() => clear()}
                    aria-label="Clear filter"
                    data-testid={props.testId ? `${props.testId}-clear` : undefined}
                >
                    &times;
                </button>
            </Show>
        </>
    );
    return props.bare ? parts : <span class={clsx("ui-filter", props.class)}>{parts}</span>;
}
