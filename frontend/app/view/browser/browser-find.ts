// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Find in page for a browser pane (Ctrl/Cmd+F): the app's find bar (element/search) in front of the page, driving
 * the host's text search of the page. The host highlights the matches in the page and answers each count update with
 * a `browser-pane-find-result` event.
 */

import { createEffect, on, onCleanup, onMount } from "solid-js";
import { useSearch } from "@/app/element/search";
import type { SearchProps } from "@/app/element/search-state";
import { getApi } from "@/util/app-api";
import type { BrowserViewModel } from "./browser-model";

/** What the host's `browser-pane-find-result` carries (client/page_find.rs): `active` is 1-based, 0 for none. Each is -1 when
 *  that update does not change it (a count-only update has no current match, a match-only one has no count). */
interface FindResult {
    block_id: string;
    count: number;
    active: number;
}

/** The anchor the bar reads when it is placed, not when the hook runs: the page area is not mounted yet then. */
const anchorRef = (anchor: () => HTMLElement | undefined) => ({
    get current() {
        return anchor() ?? null;
    },
    set current(_v: HTMLElement | null) {},
});

/** The find bar's props for this pane. `anchor` is the page area the bar floats over (top-end of it). */
export function useBrowserFind(model: BrowserViewModel, anchor: () => HTMLElement | undefined): SearchProps {
    const panes = () => getApi().browserPanes;
    const search = useSearch({ viewModel: model, anchorRef: anchorRef(anchor), caseSensitive: false });
    search.owner = model.blockId;
    let text = "";

    // The page's own search, as the person types: a new search from the top of the page each time. A closed bar
    // (the box resets to empty) ends the search at the page, which drops its highlights and selection.
    search.onSearch = (value: string) => {
        if (value === text) return;
        const hadText = text !== "";
        text = value;
        if (value === "") {
            if (hadText) void panes().stopFind(model.blockId, true).catch(() => {});
            return;
        }
        void panes().find(model.blockId, value, { matchCase: search.caseSensitive?.() ?? false }).catch(() => {});
    };
    const step = (forward: boolean) => {
        if (text === "") return;
        void panes()
            .find(model.blockId, text, { forward, findNext: true, matchCase: search.caseSensitive?.() ?? false })
            .catch(() => {});
    };
    search.onNext = () => step(true);
    search.onPrev = () => step(false);

    // Case sensitivity changes the matches: search again from the top.
    createEffect(
        on(
            () => search.caseSensitive?.(),
            (_, prev) => {
                if (prev === undefined || text === "") return;
                void panes().find(model.blockId, text, { matchCase: search.caseSensitive?.() ?? false }).catch(() => {});
            }
        )
    );

    onMount(() => {
        let unsub: (() => void) | undefined;
        void getApi()
            .listen<FindResult>("browser-pane-find-result", (r) => {
                if (r.block_id !== model.blockId || text === "") return;
                if (r.count >= 0) search.resultsCount._set(r.count);
                if (r.active >= 0) search.resultsIndex._set(Math.max(0, r.active - 1));
            })
            .then((fn) => {
                unsub = fn;
            });
        onCleanup(() => unsub?.());
    });
    // The pane closing, or the page going away with the bar open, leaves nothing to clear.
    onCleanup(() => {
        if (text !== "") void panes().stopFind(model.blockId, true).catch(() => {});
    });
    return search;
}
