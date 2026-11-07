// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's find index (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.2): the searchable text of every
 * row of the document, folded to lower case once per row and kept while the bar is open. The search reads this,
 * never the DOM, so rows the virtualiser has not mounted are found as well as the ones on screen.
 *
 * A pass walks the rows in slices, yielding between them, so a long transcript never holds the main thread for
 * more than a slice; the next query or the bar closing cancels it.
 */

import type { DocumentNode } from "../../../agent-model/types";

/** The most characters of one row that are folded and searched. */
export const FOLD_CAP = 64 * 1024;
/** How long a pass may run before it yields to the page. */
export const SLICE_MS = 8;
/** Rows between clock reads inside a slice. */
const CLOCK_EVERY = 32;

/** The text of a row the search reads: the one list that the index, the tests and the highlighter share. */
export function searchableText(node: DocumentNode): string {
    switch (node.type) {
        case "markdown":
            return node.content;
        case "user_message":
        case "agent_message":
            return node.message;
        case "tool":
            return node.tool + " " + JSON.stringify(node.params ?? {});
        case "shell":
            return node.cmd + " " + node.title;
        case "jekt_message":
            return node.from + " " + node.to + " " + node.message;
        case "system_note":
            return node.text;
        default:
            return "";
    }
}

/** What a pass needs from the page: a clock and a way to give the thread back. */
export interface PassClock {
    now: () => number;
    yieldNow: () => Promise<void>;
}

/** `scheduler.yield` where the engine has it, else a zero timeout. */
export const pageClock: PassClock = {
    now: () => performance.now(),
    yieldNow: () => {
        const scheduler = (globalThis as { scheduler?: { yield?: () => Promise<void> } }).scheduler;
        return scheduler?.yield ? scheduler.yield() : new Promise<void>((resolve) => setTimeout(resolve, 0));
    },
};

interface Folded {
    ref: DocumentNode;
    text: string;
}

export interface TranscriptIndex {
    /**
     * The ids of the rows whose text holds `query` (already lower case), in document order, or null when
     * `cancelled()` turned true during the pass. Rows unchanged since they were folded are not folded again.
     */
    search(nodes: readonly DocumentNode[], query: string, cancelled: () => boolean): Promise<string[] | null>;
    /** How many rows have been folded so far (a row folded again counts again). */
    folds(): number;
    /** Forget every row. */
    clear(): void;
}

export function createTranscriptIndex(clock: PassClock = pageClock, sliceMs: number = SLICE_MS): TranscriptIndex {
    const cache = new Map<string, Folded>();
    let folds = 0;

    const folded = (node: DocumentNode): string => {
        const hit = cache.get(node.id);
        if (hit && hit.ref === node) return hit.text;
        folds++;
        const text = searchableText(node).slice(0, FOLD_CAP).toLowerCase();
        cache.set(node.id, { ref: node, text });
        return text;
    };

    return {
        async search(nodes, query, cancelled) {
            const out: string[] = [];
            const seen = new Set<string>();
            let sliceStart = clock.now();
            for (let i = 0; i < nodes.length; i++) {
                if (i % CLOCK_EVERY === 0 && i > 0 && clock.now() - sliceStart >= sliceMs) {
                    await clock.yieldNow();
                    if (cancelled()) return null;
                    sliceStart = clock.now();
                }
                const node = nodes[i];
                seen.add(node.id);
                if (folded(node).includes(query)) out.push(node.id);
            }
            if (cancelled()) return null;
            // Rows that left the document (a different slice loaded) stop costing memory.
            for (const id of cache.keys()) if (!seen.has(id)) cache.delete(id);
            return out;
        },
        folds: () => folds,
        clear: () => cache.clear(),
    };
}

/**
 * Where stepping starts: the first match at or after the row at the top of the screen (the way an editor starts
 * from the cursor), or the first match when none is below it. `order` is the document's ids in order.
 */
export function firstMatchFrom(matches: readonly string[], order: readonly string[], topId: string | null): number {
    if (matches.length === 0) return -1;
    const top = topId == null ? -1 : order.indexOf(topId);
    if (top < 0) return 0;
    const position = new Map<string, number>();
    for (let i = top; i < order.length; i++) position.set(order[i], i);
    const at = matches.findIndex((id) => position.has(id));
    return at < 0 ? 0 : at;
}

/**
 * The document row to start from for the shown row `topId`: the screen shows hidden-run lines and day dividers that
 * the document does not hold under their own ids, so a run stands for its first hidden row and a divider for the
 * next row that is in the document. Null when `topId` is not shown or nothing at or after it is in the document.
 */
export function rawIdAtOrAfter(
    shown: readonly DocumentNode[],
    topId: string | null,
    raw: ReadonlySet<string>
): string | null {
    const start = topId === null ? -1 : shown.findIndex((n) => n.id === topId);
    if (start < 0) return null;
    for (let i = start; i < shown.length; i++) {
        const node = shown[i];
        if (node.type === "hidden_run") {
            const held = node.hiddenIds.find((id) => raw.has(id));
            if (held !== undefined) return held;
        } else if (raw.has(node.id)) return node.id;
    }
    return null;
}

/** The id of the row at the top of the screen: the first whose bottom edge is below `scrollTop`. */
export function topRowId(rows: readonly { nodeId: string; end: number }[], scrollTop: number): string | null {
    for (const row of rows) if (row.end > scrollTop) return row.nodeId;
    return null;
}
