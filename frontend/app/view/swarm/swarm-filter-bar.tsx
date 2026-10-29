// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Swarm's filter bar and the highlight of what matched (docs/specs/SPEC_SWARM_FILTER_BAR_2026_10_29.md §2). */

import { FilterInput } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";
import { splitMatches } from "./swarm-filter";
import type { SwarmFilterState } from "./swarm-filter-state";

/** `text` with the parts that match `words` marked; the same text when there are none. */
export function Highlight(props: { text: string; words: readonly string[] }): JSX.Element {
    return (
        <For each={splitMatches(props.text, props.words)}>
            {(piece) => (piece.match ? <mark class="swarm-match">{piece.text}</mark> : piece.text)}
        </For>
    );
}

/** The bar's focus handle, so the pane's Find (Ctrl/Cmd+F) reaches the box. */
export interface SwarmFilterFocus {
    focus(): boolean;
}

export function SwarmFilterBar(props: { filter: SwarmFilterState; focusRef?: (focus: SwarmFilterFocus) => void }): JSX.Element {
    const filter = props.filter;
    let bar: HTMLDivElement | undefined;
    props.focusRef?.({
        focus: () => {
            const input = bar?.querySelector<HTMLInputElement>("input");
            input?.focus();
            input?.select();
            return !!input;
        },
    });
    const filtering = () => filter.text().trim() !== "";
    return (
        <div
            class="swarm-filter-bar"
            data-testid="swarm-filter-bar"
            ref={bar}
            onKeyDown={(e) => {
                // FilterInput clears on the first Escape; a second one, with the box empty, leaves it.
                if (e.key === "Escape" && !e.defaultPrevented) (e.target as HTMLElement).blur();
            }}
        >
            <FilterInput
                class="swarm-filter-input"
                value={filter.text()}
                onInput={filter.setText}
                onClear={filter.clear}
                placeholder="Filter agents and work…"
                ariaLabel="Filter agents by name or what they are working on"
                testId="swarm-filter"
            />
            <Show when={filtering()}>
                <span class="swarm-filter-count" data-testid="swarm-filter-count" title="Agents shown of the agents listed">
                    {filter.counts().shown} of {filter.counts().total}
                </span>
            </Show>
        </div>
    );
}

/** The list's note when the filter leaves nothing. */
export function SwarmFilterEmpty(props: { filter: SwarmFilterState }): JSX.Element {
    return (
        <div class="swarm-filter-empty" data-testid="swarm-filter-empty">
            <span>No agents match “{props.filter.text().trim()}”</span>
            <button type="button" class="swarm-filter-empty-clear" onClick={() => props.filter.clear()}>
                Clear
            </button>
        </div>
    );
}
