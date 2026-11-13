// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the transcript reads of the search to paint it (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.4). The
 * pane provides it around the transcript, so the list and its rows read it without a prop through every layer.
 * A row asks one boolean of the match set (by node id, so a row the virtualiser recycles is right on mount), the
 * scroller says whether dim applies, and the highlighter reads the query and the current row.
 */

import { createContext, useContext, type Accessor } from "solid-js";
import type { UseInSessionSearch } from "../hooks/useInSessionSearch";

export interface SearchPaint {
    /** The ids of the rows that match. */
    matches: Accessor<ReadonlySet<string>>;
    /** The query (lower case) whose matches are showing; "" when no search is showing. */
    searched: Accessor<string>;
    /** Non-matching rows are dimmed: the bar's dim switch is on and a query is showing. */
    dim: Accessor<boolean>;
    /** The row the search is on, if any. */
    currentId: Accessor<string | null>;
}

export const SearchPaintContext = createContext<SearchPaint>();

/** The pane's search paint, or undefined where none is provided (a row outside a pane with search). */
export const useSearchPaint = (): SearchPaint | undefined => useContext(SearchPaintContext);

export function createSearchPaint(
    search: Pick<UseInSessionSearch, "matchSet" | "searched" | "dim" | "highlightId">
): SearchPaint {
    return {
        matches: search.matchSet,
        searched: search.searched,
        dim: () => search.dim() && search.searched() !== "",
        currentId: search.highlightId,
    };
}
