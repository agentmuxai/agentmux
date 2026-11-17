// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignalAtom } from "@/util/util";
import { carryPaneScope } from "@/app/backdrop/overlay-scope";
import { JSX, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { SearchBar } from "./search-parts";
import { createSearch, type SearchProps } from "./search-state";
import "./search.scss";

const SearchComponent = (props: SearchProps): JSX.Element => {
    const s = createSearch(props);
    return (
        <Show when={s.isOpen()}>
            {/* The bar takes the colours of the pane it searches (a pane with a backdrop). They go on the portal's own
                element: the bar's style is rewritten whole each time it is placed. */}
            <Portal
                ref={(mount) => {
                    const stop = carryPaneScope(mount, props.anchorRef?.current);
                    if (stop) onCleanup(stop);
                }}
            >
                <SearchBar s={s} />
            </Portal>
        </Show>
    );
};

export const Search = SearchComponent;

type SearchOptions = {
    anchorRef?: { current: HTMLElement | null };
    /** Gets the find bar, so the host can open and close it (Ctrl+F /
     *  Escape). A native pane tab hands it over as `PaneTabInstance.search`. */
    viewModel?: { searchAtoms?: SearchAtoms };
    regex?: boolean;
    caseSensitive?: boolean;
    wholeWord?: boolean;
};

export function useSearch(options?: SearchOptions): SearchProps {
    const searchAtoms: SearchAtoms = {
        searchValue: createSignalAtom(""),
        resultsIndex: createSignalAtom(0),
        resultsCount: createSignalAtom(0),
        isOpen: createSignalAtom(false),
        regex: options?.regex !== undefined ? createSignalAtom(options.regex) : undefined,
        caseSensitive: options?.caseSensitive !== undefined ? createSignalAtom(options.caseSensitive) : undefined,
        wholeWord: options?.wholeWord !== undefined ? createSignalAtom(options.wholeWord) : undefined,
    };

    const anchorRef = options?.anchorRef ?? { current: null };

    if (options?.viewModel) {
        options.viewModel.searchAtoms = searchAtoms;
    }

    return { ...searchAtoms, anchorRef };
}
