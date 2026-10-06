// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared fuzzy/relevance-ranked search, reused across every text-search UI in
// the app (command palette, agent-picker filter, Settings search — see
// docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md). A plain function, not
// a stateful class or Solid primitive: every consumer already holds its own
// corpus behind a createMemo/signal, so this only needs to do the matching.
//
// Fuzzy matching (typo/reorder tolerance) and synonym matching are different
// problems — this utility only solves the former. "Synonyms match" comes from
// a domain-specific `keywords` field a caller searches over as one of its
// `keys` (see the Settings consumer), authored as curated static data, not
// from anything in here.

import Fuse, { type IFuseOptions } from "fuse.js";

// Tuned once for "interactive search over a small, known corpus" — permissive
// enough for typos without being noisy, matches anywhere in the string (not
// just the start). Kept consistent app-wide so search *feels* the same
// everywhere; callers override/extend `keys` per domain, which is the part
// that's genuinely domain-specific.
export const DEFAULT_FUZZY_OPTIONS: Partial<IFuseOptions<unknown>> = {
    threshold: 0.35,
    ignoreLocation: true,
    // 1, not e.g. 2: every consumer this utility replaces (command palette,
    // agent-picker filter) used a bare `.includes()` with no length floor —
    // a 1-character query matched fine. A higher floor here would be a
    // silent behavior regression for short queries (caught live by
    // MyAgentsList.test.tsx's "m" → "Maks" case during the migration this
    // utility was built for).
    minMatchCharLength: 1,
};

/**
 * Fuzzy-search `items` for `query`, ranked by relevance. Returns `items`
 * unchanged (original order) when `query` is blank — "not searching," not
 * "searching, zero results."
 *
 * Builds a fresh Fuse index per call: every corpus in this app today
 * (commands ~25, settings ~59, an individual user's agent list) is small
 * enough that this isn't a measurable cost. If a future consumer's corpus
 * grows large enough to change that calculus, hold a memoized `Fuse`
 * instance directly instead of reaching for this convenience wrapper — it's
 * a thin function, not a required abstraction layer.
 */
export function fuzzySearch<T>(items: T[], query: string, options: IFuseOptions<T>): T[] {
    const q = query.trim();
    if (!q) return items;
    const fuse = new Fuse(items, { ...DEFAULT_FUZZY_OPTIONS, ...options });
    return fuse.search(q).map((r) => r.item);
}

/**
 * Search where what the user typed counts first. Names that contain `query`
 * (case-insensitive) are returned, best first: the exact name, then names that
 * start with it, then names with it at a word start, then the rest, each group
 * in `items` order. Only when no name contains it does this fall back to
 * `fuzzySearch`, so a typo still finds something.
 *
 * `fuzzySearch` alone treats "agentx" as close enough to "agenty", which is
 * right for a command palette and wrong for a list the user is narrowing by a
 * name they know. Returns `items` unchanged for a blank query.
 */
export function literalFirstSearch<T>(
    items: T[],
    query: string,
    text: (item: T) => string,
    fuzzyOptions: IFuseOptions<T>
): T[] {
    const q = query.trim();
    if (!q) return items;
    const needle = q.toLowerCase();
    const hits: { item: T; rank: number; index: number }[] = [];
    items.forEach((item, index) => {
        const haystack = text(item).toLowerCase();
        const at = haystack.indexOf(needle);
        if (at < 0) return;
        const rank = haystack === needle ? 0 : at === 0 ? 1 : /[\s\-_.]/.test(haystack[at - 1]) ? 2 : 3;
        hits.push({ item, rank, index });
    });
    if (hits.length > 0) return hits.sort((a, b) => a.rank - b.rank || a.index - b.index).map((h) => h.item);
    return fuzzySearch(items, q, fuzzyOptions);
}
