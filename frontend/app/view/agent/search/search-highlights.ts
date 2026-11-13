// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words a search finds, painted with the CSS Custom Highlight API (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06
 * §4.4): `agent-search` for every occurrence in a mounted matching row and `agent-search-current` for those in the
 * current row, coloured in styles/_search.scss. A mounted row walks its text nodes after it renders and on every change
 * to them, finds the query with the same fold as the index (`toLowerCase`), and takes its ranges back when it
 * unmounts or the query changes. A match that spans nodes (a word split by emphasis) is not painted; the row is still
 * counted and current. Where `CSS.highlights` is missing nothing is painted and the row treatment stands alone.
 */

import { createEffect, onCleanup } from "solid-js";
import type { SearchPaint } from "./search-paint";

export const HIGHLIGHT_ALL = "agent-search";
export const HIGHLIGHT_CURRENT = "agent-search-current";
/** A row paints at most this many words; the rest of a very long one is still counted by the index. */
export const MAX_RANGES_PER_ROW = 500;

/** The page's highlight registry, or null where the API is missing (looked up each time so a test can stub it). */
function registry(): HighlightRegistry | null {
    return typeof CSS !== "undefined" && CSS.highlights && typeof Highlight !== "undefined" ? CSS.highlights : null;
}

function bucket(reg: HighlightRegistry, name: string): Highlight {
    let found = reg.get(name);
    if (!found) {
        found = new Highlight();
        reg.set(name, found);
    }
    return found;
}

/** The ranges of `needle` (already folded) in the text nodes under `root`. */
export function findRanges(root: HTMLElement, needle: string): Range[] {
    const ranges: Range[] = [];
    if (!needle) return ranges;
    const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node && ranges.length < MAX_RANGES_PER_ROW; node = walker.nextNode()) {
        const text = (node as Text).data;
        const folded = text.toLowerCase();
        // A fold that changes the length (a few letters do) would put the offsets in the wrong place.
        if (folded.length !== text.length) continue;
        for (let at = folded.indexOf(needle); at >= 0 && ranges.length < MAX_RANGES_PER_ROW; at = folded.indexOf(needle, at + needle.length)) {
            const range = root.ownerDocument.createRange();
            range.setStart(node, at);
            range.setEnd(node, at + needle.length);
            ranges.push(range);
        }
    }
    return ranges;
}

/** Paint `ranges` as the all-words or the current-row highlight; the returned function takes them back. */
export function paintRanges(ranges: Range[], current: boolean): () => void {
    const reg = registry();
    if (!reg || ranges.length === 0) return () => {};
    const set = bucket(reg, current ? HIGHLIGHT_CURRENT : HIGHLIGHT_ALL);
    for (const r of ranges) set.add(r);
    return () => {
        for (const r of ranges) set.delete(r);
    };
}

/**
 * Keep a mounted row's words painted while it matches. `isMatch` is the row's one boolean; the row's text can change
 * without a signal (markdown renders late, a tool opens), so a change under it repaints on the next frame.
 */
export function createRowHighlights(
    root: () => HTMLElement | undefined,
    paint: SearchPaint,
    nodeId: () => string,
    isMatch: () => boolean
): void {
    createEffect(() => {
        const el = root();
        const needle = paint.searched();
        if (!el || !needle || !isMatch() || !registry()) return;
        const current = paint.currentId() === nodeId();
        let release = paintRanges(findRanges(el, needle), current);
        let frame = 0;
        const repaint = () => {
            frame = 0;
            release();
            release = paintRanges(findRanges(el, needle), current);
        };
        const observer = new MutationObserver(() => {
            if (!frame) frame = requestAnimationFrame(repaint);
        });
        observer.observe(el, { childList: true, subtree: true, characterData: true });
        onCleanup(() => {
            observer.disconnect();
            if (frame) cancelAnimationFrame(frame);
            release();
        });
    });
}
