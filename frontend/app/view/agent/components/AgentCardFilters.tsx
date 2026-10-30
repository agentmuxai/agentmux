// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCardFilters: the Free, Local and Installed chips above the new-agent
 * cards, narrowing the grid (more than one on keeps only a card that passes
 * them all). Each chip's count is what selecting it would leave, so it follows
 * the others. Not remembered between visits: a hidden-by-default filter is how
 * a card seems to disappear. A polite live region says how many are shown.
 * docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §6.
 */

import { Show, type JSX } from "solid-js";
import { Button, FilterChip, FilterChips } from "@/app/element/ui";
import { CARD_FILTERS, filterCounts, type CardFilter, type FilterableCard } from "../defaults/provider-access-model";

export interface AgentCardFiltersProps {
    /** Every template card, before filtering, with what the install probe knows. */
    cards: readonly FilterableCard[];
    selected: readonly CardFilter[];
    onChange: (next: CardFilter[]) => void;
    /** How many cards pass the selected filters. */
    shown: number;
    /** Every card's install probe has answered, so an Installed count is not a wrong zero. */
    installedKnown: boolean;
    /** Replaceable in tests; the date the freshness rules read. */
    now?: () => Date;
}

const LABEL: Record<CardFilter, string> = { free: "Free", local: "Local", installed: "Installed" };
const ICON: Partial<Record<CardFilter, string>> = { free: "gift", local: "house" };

export const AgentCardFilters = (props: AgentCardFiltersProps): JSX.Element => {
    const counts = () => filterCounts(props.cards, new Set(props.selected), (props.now ?? (() => new Date()))());
    const isFilter = (id: string): id is CardFilter => (CARD_FILTERS as readonly string[]).includes(id);
    return (
        <div class="agent-card-filters" data-testid="agent-card-filters">
            <FilterChips
                value={props.selected}
                onChange={(next) => props.onChange(next.filter(isFilter))}
                aria-label="Filter the new-agent cards"
            >
                {CARD_FILTERS.map((f) => (
                    <FilterChip
                        id={f}
                        icon={ICON[f]}
                        count={f === "installed" && !props.installedKnown ? undefined : counts()[f]}
                    >
                        {LABEL[f]}
                    </FilterChip>
                ))}
            </FilterChips>
            <span class="agent-card-filters-status" role="status" aria-live="polite" data-testid="agent-card-filters-status">
                <Show when={props.selected.length > 0}>
                    {props.shown} of {props.cards.length} shown
                </Show>
            </span>
            <Show when={props.selected.length > 0 && props.shown === 0}>
                <p class="agent-card-filters-empty" data-testid="agent-card-filters-empty">
                    No agent is {props.selected.map((f) => LABEL[f].toLowerCase()).join(" and ")}.{" "}
                    <Button tone="quiet" onClick={() => props.onChange([])} data-testid="agent-card-filters-clear">
                        Clear filters
                    </Button>
                </p>
            </Show>
        </div>
    );
};

AgentCardFilters.displayName = "AgentCardFilters";
