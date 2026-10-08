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
 * `text` gives an item's name, or a list: its name first, then other words it
 * is known by (keywords, a category, an id, a description; blanks are
 * skipped). Every item whose name contains `query` comes before every item
 * that only matches in those other words; within each group the order above
 * applies, by the item's best string.
 *
 * `fuzzySearch` alone treats "agentx" as close enough to "agenty", which is
 * wrong when the user types a name they know: an exact name should come first,
 * not share the list with near-misses. Returns `items` unchanged for a blank
 * query.
 */
export function literalFirstSearch<T>(
    items: T[],
    query: string,
    text: (item: T) => string | readonly (string | null | undefined)[],
    fuzzyOptions: IFuseOptions<T>
): T[] {
    const q = query.trim();
    if (!q) return items;
    const needle = q.toLowerCase();
    const hits: { item: T; rank: number; index: number }[] = [];
    items.forEach((item, index) => {
        const texts = text(item);
        const [name, ...others] = typeof texts === "string" ? [texts] : texts;
        let rank = containsRank(name, needle);
        if (rank < 0) {
            for (const other of others) {
                const r = containsRank(other, needle);
                if (r >= 0 && (rank < 0 || r < rank)) rank = r;
            }
            if (rank >= 0) rank += OTHER_WORDS_RANK;
        }
        if (rank >= 0) hits.push({ item, rank, index });
    });
    if (hits.length > 0) return hits.sort((a, b) => a.rank - b.rank || a.index - b.index).map((h) => h.item);
    return fuzzySearch(items, q, fuzzyOptions);
}

// Added to a match in an item's other words, so it ranks below any name match
// (whose ranks are 0..3).
const OTHER_WORDS_RANK = 4;

// How `haystack` contains `needle` (already lower-case): 0 is the whole
// string, 1 at its start, 2 at a word start, 3 elsewhere, -1 not at all.
function containsRank(haystack: string | null | undefined, needle: string): number {
    if (!haystack) return -1;
    const h = haystack.toLowerCase();
    const at = h.indexOf(needle);
    if (at < 0) return -1;
    return h === needle ? 0 : at === 0 ? 1 : /[\s\-_.]/.test(h[at - 1]) ? 2 : 3;
}

/**
 * The plain "type to narrow" match the panes' filter boxes share (FilterInput):
 * every word of `query` appears, case-insensitively, in one of `texts`. No
 * ranking and no fuzziness, so what shows stays in its order and predictable;
 * use `literalFirstSearch` where results should be ranked. A blank query
 * matches everything.
 */
export function matchesEveryWord(query: string, ...texts: (string | null | undefined)[]): boolean {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (words.length === 0) return true;
    const haystack = texts
        .filter((t): t is string => !!t)
        .map((t) => t.toLowerCase())
        .join("\n");
    return words.every((w) => haystack.includes(w));
}
