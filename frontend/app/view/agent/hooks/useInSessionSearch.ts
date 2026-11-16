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
 *   - matchSet         — the matches as a Set, for a row to ask "am I one?" (search/search-paint.ts)
 *   - searched         — the query (lower case) whose matches are showing; "" when there is none
 *   - text             — what is typed in the bar, as typed; undefined until something is typed or the bar is brought
 *                        back after a recycle (a bar with text keeps the caret where the person had it)
 *   - dim, setDim      — whether non-matching rows are dimmed while a query is showing (on by default)
 *   - searching        — a pass is taking long enough to say so
 *   - performSearch    — run a fresh query (after a short pause in typing)
 *   - next             — cycle to next match
 *   - prev             — cycle to previous match
 *   - close            — clear matches and hide the bar
 *
 * This hook doesn't own scroll. Callers pass a
 * `jumpTo(nodeId)` callback that gets invoked on every navigation.
 */

import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor, type Setter } from "solid-js";
import type { DocumentNode } from "../../../agent-model/types";
import { registerSearchForReload, takeStoredSearch, type StoredSearch } from "../search/search-reload";
import { createTranscriptIndex, firstMatchFrom, type PassClock } from "../search/transcript-index";

/** Typing pauses this long before a pass runs. */
export const SEARCH_DEBOUNCE_MS = 120;
/** A growing transcript is searched again at most this often. */
export const SEARCH_REFRESH_MS = 200;
/** A pass that takes longer than this shows "Searching…". */
export const SEARCH_SLOW_MS = 150;

export interface UseInSessionSearchOptions {
    /**
     * The pane's block. With it, an open bar is written down when the window recycles and put back, with its text and
     * current match, when the pane loads again (search/search-reload.ts); without it the bar is forgotten.
     */
    blockId?: string;
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
    matchSet: Accessor<ReadonlySet<string>>;
    searched: Accessor<string>;
    /** What is typed in the bar, as typed; undefined for a bar nobody has typed in or brought back (it takes the caret). */
    text: Accessor<string | undefined>;
    dim: Accessor<boolean>;
    setDim: Setter<boolean>;
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
    setSearched: Setter<string>;
    setText: Setter<string | undefined>;
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
    // A refresh of a growing transcript never moves the view, and never marks a row it did not move to: with no
    // current match, the new matches are counted and the first step picks one.
    if (!typed) {
        view.setCurrentIndex(-1);
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
    opts.jumpTo?.(found[start]);
}

/** One try at the match a brought-back bar had: it becomes current when this pass has it. Spent either way. */
function takeSeed(view: SearchView, found: string[], seed: string): null {
    const at = found.indexOf(seed);
    if (at >= 0) view.setCurrentIndex(at);
    return null;
}

/**
 * Rows that arrive or grow while the bar is open join the results, at most every SEARCH_REFRESH_MS. Returns what
 * cancels a refresh that is waiting.
 */
function refreshWhileGrowing(
    opts: UseInSessionSearchOptions,
    visible: Accessor<boolean>,
    query: () => string,
    run: (typed: boolean) => Promise<void>
): () => void {
    let refresh: ReturnType<typeof setTimeout> | undefined;
    createEffect(
        on(
            () => opts.document(),
            () => {
                if (!visible() || !query() || refresh !== undefined) return;
                refresh = setTimeout(() => {
                    refresh = undefined;
                    if (visible() && query()) void run(false);
                }, SEARCH_REFRESH_MS);
            },
            { defer: true }
        )
    );
    return () => {
        clearTimeout(refresh);
        refresh = undefined;
    };
}

/** The debounced, sliced, cancellable passes over the transcript, and the refresh while it grows. */
function createSearchPasses(opts: UseInSessionSearchOptions, view: SearchView) {
    const { visible, highlightId, setMatches, setCurrentIndex, setSearching, setSearched, setText } = view;
    const index = createTranscriptIndex(opts.clock);
    /** The query as searched (lower case); empty when there is none. */
    let query = "";
    /** The current match of a bar brought back after a recycle: taken up by the first pass that finds anything. */
    let seed: string | null = null;
    /** Bumped by every pass and by the search ending: a pass that finds it changed is stale and stops. */
    let pass = 0;
    /** A typed query's pass was cut off by a newer one: whoever finishes is owed its jump. */
    let typedPending = false;
    let debounce: ReturnType<typeof setTimeout> | undefined;
    let slow: ReturnType<typeof setTimeout> | undefined;

    const run = async (typed: boolean): Promise<void> => {
        const mine = ++pass;
        typedPending ||= typed;
        clearTimeout(slow);
        if (!query) {
            setSearching(false);
            typedPending = false;
            setSearched("");
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
        setSearched(query);
        applyFound(opts, view, found, owed, nodes);
        // Rows that load later do not pull the bar's old current match back.
        if (seed !== null && found.length > 0) seed = takeSeed(view, found, seed);
    };

    const performSearch = (text: string) => {
        seed = null;
        setText(text);
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

    const cancelRefresh = refreshWhileGrowing(opts, visible, () => query, run);

    /** Run a pass that the pause in typing has not yet started: a step asked for now means "now". */
    const flushTyping = (): boolean => {
        if (debounce === undefined) return false;
        clearTimeout(debounce);
        debounce = undefined;
        void run(true);
        return true;
    };

    /** Bring a bar back as it was: its text searched again, quietly (no jump), with its current match when found. */
    const restore = (text: string, matchId: string | null) => {
        setText(text);
        query = text.trim() ? text.toLowerCase() : "";
        seed = matchId;
        if (query) void run(false);
    };

    const stop = () => {
        pass++;
        typedPending = false;
        seed = null;
        query = "";
        clearTimeout(debounce);
        cancelRefresh();
        clearTimeout(slow);
        debounce = undefined;
        setSearching(false);
        setSearched("");
        setText(undefined);
    };

    return { performSearch, restore, flushTyping, stop, clearIndex: index.clear };
}

export function useInSessionSearch(opts: UseInSessionSearchOptions): UseInSessionSearch {
    // A bar that a recycle wrote down comes back open, as it was (search/search-reload.ts).
    const restored: StoredSearch | undefined = opts.blockId ? takeStoredSearch(opts.blockId) : undefined;
    const [visible, setVisible] = createSignal(restored !== undefined);
    const [matches, setMatches] = createSignal<string[]>([]);
    const [currentIndex, setCurrentIndex] = createSignal(-1);
    const [searching, setSearching] = createSignal(false);
    const [searched, setSearched] = createSignal("");
    const [text, setText] = createSignal<string | undefined>(restored?.text);
    const [dim, setDim] = createSignal(restored?.dim ?? true);

    const matchCount = createMemo(() => matches().length);

    const highlightId = createMemo<string | null>(() => {
        const m = matches();
        const idx = currentIndex();
        return idx >= 0 && idx < m.length ? m[idx] : null;
    });

    const matchSet = createMemo<ReadonlySet<string>>(() => new Set(matches()));

    const { performSearch, restore, flushTyping, stop, clearIndex } = createSearchPasses(opts, {
        visible,
        highlightId,
        setMatches,
        setCurrentIndex,
        setSearching,
        setSearched,
        setText,
    });
    onCleanup(stop);
    if (restored) restore(restored.text, restored.matchId);
    // An open bar is no reason to keep a window at its size: a recycle writes it down, and the reloaded pane reads it.
    if (opts.blockId) {
        onCleanup(
            registerSearchForReload(opts.blockId, () =>
                visible() ? { text: text() ?? "", matchId: highlightId(), dim: dim() } : null
            )
        );
    }

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
        matchSet,
        searched,
        text,
        dim,
        setDim,
        searching,
        performSearch,
        next,
        prev,
        close,
    };
}
