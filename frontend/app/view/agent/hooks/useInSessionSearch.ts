// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useInSessionSearch — owns the Ctrl+F search state for the agent
 * pane: query results, current match, navigation, highlight.
 *
 * Step 8 of docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md; the index, the sliced pass and the
 * streaming refresh are SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.2 (search/transcript-index.ts).
 *
 * Searches over the currently-loaded document slice only. Searching
 * the full persisted history would require a backend
 * blockfile:search RPC — out of scope for this hook.
 *
 * Returns:
 *   - visible          — whether the search bar is shown
 *   - setVisible       — toggle the bar (Setter so the Ctrl+F handler
 *                        can use functional updaters)
 *   - matches          — current matched node IDs, in document order
 *   - currentIndex     — 0-based index into matches; -1 when none
 *   - matchCount       — derived: matches().length
 *   - highlightId      — derived: matches[currentIndex] or null
 *   - searching        — a pass is taking long enough to say so
 *   - performSearch    — run a fresh query (after a short pause in typing)
 *   - next             — cycle to next match
 *   - prev             — cycle to previous match
 *   - close            — clear matches and hide the bar
 *
 * This hook doesn't own scroll. Callers pass a
 * `jumpTo(nodeId)` callback that gets invoked on every navigation.
 */

import { registerRecycleBlocker } from "@/app/store/recycle-blockers";
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor, type Setter } from "solid-js";
import type { DocumentNode } from "../../../agent-model/types";
import { createTranscriptIndex, firstMatchFrom, type PassClock } from "../search/transcript-index";

/** Typing pauses this long before a pass runs. */
export const SEARCH_DEBOUNCE_MS = 120;
/** A growing transcript is searched again at most this often. */
export const SEARCH_REFRESH_MS = 200;
/** A pass that takes longer than this shows "Searching…". */
export const SEARCH_SLOW_MS = 150;

export interface UseInSessionSearchOptions {
    /**
     * Reactive accessor for the document slice to search over.
     * Pass the document atom's read accessor.
     */
    document: Accessor<DocumentNode[]>;
    /**
     * Called with the matched node id when the user navigates to a
     * result (initial match, next, prev). Optional — search can
     * function without a scroll target.
     */
    jumpTo?: (nodeId: string) => void;
    /** Called when the search ends (the bar closes), after its matches are cleared. */
    onClose?: () => void;
    /** The row at the top of the screen: a new query starts at the first match from there. */
    topNodeId?: () => string | null;
    /** The search has no current match any more (an empty query, or none found). */
    onNoMatch?: () => void;
    /** Clock and yield for the pass; the page's own by default. */
    clock?: PassClock;
}

export interface UseInSessionSearch {
    visible: Accessor<boolean>;
    setVisible: Setter<boolean>;
    matches: Accessor<string[]>;
    currentIndex: Accessor<number>;
    matchCount: Accessor<number>;
    highlightId: Accessor<string | null>;
    searching: Accessor<boolean>;
    performSearch: (query: string) => void;
    next: () => void;
    prev: () => void;
    close: () => void;
}

/** What the passes read and write of the hook's state. */
interface SearchView {
    visible: Accessor<boolean>;
    highlightId: Accessor<string | null>;
    setMatches: Setter<string[]>;
    setCurrentIndex: Setter<number>;
    setSearching: Setter<boolean>;
}

/** Take a pass's matches in: keep the current one by id while it still matches, else pick a start; move the view only for a typed query. */
function applyFound(
    opts: UseInSessionSearchOptions,
    view: SearchView,
    found: string[],
    typed: boolean,
    nodes: readonly DocumentNode[]
): void {
    const held = view.highlightId();
    view.setMatches(found);
    if (found.length === 0) {
        view.setCurrentIndex(-1);
        opts.onNoMatch?.();
        return;
    }
    // The current match is held by id: rows added before it, or the query growing, leave it where it is.
    const keep = held === null ? -1 : found.indexOf(held);
    if (keep >= 0) {
        view.setCurrentIndex(keep);
        return;
    }
    const top = opts.topNodeId?.() ?? null;
    const start =
        top === null
            ? 0
            : firstMatchFrom(
                  found,
                  nodes.map((n) => n.id),
                  top
              );
    view.setCurrentIndex(start);
    // A refresh of a growing transcript never moves the view; a query the person typed does.
    if (typed) opts.jumpTo?.(found[start]);
}

/** The debounced, sliced, cancellable passes over the transcript, and the refresh while it grows. */
function createSearchPasses(opts: UseInSessionSearchOptions, view: SearchView) {
    const { visible, highlightId, setMatches, setCurrentIndex, setSearching } = view;
    const index = createTranscriptIndex(opts.clock);
    /** The query as searched (lower case); empty when there is none. */
    let query = "";
    /** Bumped by every pass and by the search ending: a pass that finds it changed is stale and stops. */
    let pass = 0;
    /** A typed query's pass was cut off by a newer one: whoever finishes is owed its jump. */
    let typedPending = false;
    let debounce: ReturnType<typeof setTimeout> | undefined;
    let refresh: ReturnType<typeof setTimeout> | undefined;
    let slow: ReturnType<typeof setTimeout> | undefined;

    const run = async (typed: boolean): Promise<void> => {
        const mine = ++pass;
        typedPending ||= typed;
        clearTimeout(slow);
        if (!query) {
            setSearching(false);
            typedPending = false;
            applyFound(opts, view, [], true, []);
            return;
        }
        slow = setTimeout(() => {
            if (mine === pass) setSearching(true);
        }, SEARCH_SLOW_MS);
        const nodes = opts.document();
        const found = await index.search(nodes, query, () => mine !== pass);
        if (found === null || mine !== pass) return;
        clearTimeout(slow);
        setSearching(false);
        const owed = typedPending;
        typedPending = false;
        applyFound(opts, view, found, owed, nodes);
    };

    const performSearch = (text: string) => {
        query = text.trim() ? text.toLowerCase() : "";
        clearTimeout(debounce);
        if (!query) {
            void run(true);
            return;
        }
        debounce = setTimeout(() => {
            debounce = undefined;
            void run(true);
        }, SEARCH_DEBOUNCE_MS);
    };

    // Rows that arrive or grow while the bar is open join the results, at most every SEARCH_REFRESH_MS.
    createEffect(
        on(
            () => opts.document(),
            () => {
                if (!visible() || !query || refresh !== undefined) return;
                refresh = setTimeout(() => {
                    refresh = undefined;
                    if (visible() && query) void run(false);
                }, SEARCH_REFRESH_MS);
            },
            { defer: true }
        )
    );

    /** Run a pass that the pause in typing has not yet started: a step asked for now means "now". */
    const flushTyping = (): boolean => {
        if (debounce === undefined) return false;
        clearTimeout(debounce);
        debounce = undefined;
        void run(true);
        return true;
    };

    const stop = () => {
        pass++;
        typedPending = false;
        query = "";
        clearTimeout(debounce);
        clearTimeout(refresh);
        clearTimeout(slow);
        debounce = refresh = undefined;
        setSearching(false);
    };

    return { performSearch, flushTyping, stop, clearIndex: index.clear };
}

export function useInSessionSearch(opts: UseInSessionSearchOptions): UseInSessionSearch {
    const [visible, setVisible] = createSignal(false);
    onCleanup(registerRecycleBlocker(() => (visible() ? "a transcript search is open" : null)));
    const [matches, setMatches] = createSignal<string[]>([]);
    const [currentIndex, setCurrentIndex] = createSignal(-1);
    const [searching, setSearching] = createSignal(false);

    const matchCount = createMemo(() => matches().length);

    const highlightId = createMemo<string | null>(() => {
        const m = matches();
        const idx = currentIndex();
        return idx >= 0 && idx < m.length ? m[idx] : null;
    });

    const { performSearch, flushTyping, stop, clearIndex } = createSearchPasses(opts, {
        visible,
        highlightId,
        setMatches,
        setCurrentIndex,
        setSearching,
    });
    onCleanup(stop);

    const next = () => {
        if (flushTyping()) return;
        const m = matches();
        if (m.length === 0) return;
        const n = (currentIndex() + 1) % m.length;
        setCurrentIndex(n);
        opts.jumpTo?.(m[n]);
    };

    const prev = () => {
        if (flushTyping()) return;
        const m = matches();
        if (m.length === 0) return;
        const p = currentIndex() < 0 ? m.length - 1 : (currentIndex() - 1 + m.length) % m.length;
        setCurrentIndex(p);
        opts.jumpTo?.(m[p]);
    };

    const close = () => {
        stop();
        clearIndex();
        setVisible(false);
        setMatches([]);
        setCurrentIndex(-1);
        opts.onClose?.();
    };

    return {
        visible,
        setVisible,
        matches,
        currentIndex,
        matchCount,
        highlightId,
        searching,
        performSearch,
        next,
        prev,
        close,
    };
}
