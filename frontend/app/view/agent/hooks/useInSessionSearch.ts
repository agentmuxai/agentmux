// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useInSessionSearch — owns the Ctrl+F search state for the agent
 * pane: query results, current match index, navigation, highlight.
 *
 * Step 8 of docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Searches over the currently-loaded document slice only. Searching
 * the full persisted history would require a backend
 * blockfile:search RPC — out of scope for this hook.
 *
 * Returns:
 *   - visible          — whether the search bar is shown
 *   - setVisible       — toggle the bar (Setter so the Ctrl+F handler
 *                        can use functional updaters)
 *   - matches          — current matched node IDs
 *   - currentIndex     — 0-based index into matches; -1 when none
 *   - matchCount       — derived: matches().length
 *   - highlightId      — derived: matches[currentIndex] or null
 *   - performSearch    — run a fresh query
 *   - next             — cycle to next match
 *   - prev             — cycle to previous match
 *   - close            — clear matches and hide the bar
 *
 * This hook doesn't own scroll. Callers pass a
 * `jumpTo(nodeId)` callback that gets invoked on every navigation.
 */

import { createMemo, createSignal, type Accessor, type Setter } from "solid-js";
import type { DocumentNode } from "../types";

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
}

export interface UseInSessionSearch {
    visible: Accessor<boolean>;
    setVisible: Setter<boolean>;
    matches: Accessor<string[]>;
    currentIndex: Accessor<number>;
    matchCount: Accessor<number>;
    highlightId: Accessor<string | null>;
    performSearch: (query: string) => void;
    next: () => void;
    prev: () => void;
    close: () => void;
}

/** Extract searchable plain text from any document node. */
function nodeSearchText(node: DocumentNode): string {
    switch (node.type) {
        case "markdown":      return node.content;
        case "user_message":  return node.message;
        case "agent_message": return node.message;
        case "tool":          return node.tool + " " + JSON.stringify(node.params ?? {});
        case "section":       return node.title;
        case "shell":         return node.cmd + " " + node.title;
        case "jekt_message":  return node.from + " " + node.to + " " + node.message;
        default:              return "";
    }
}

export function useInSessionSearch(opts: UseInSessionSearchOptions): UseInSessionSearch {
    const [visible, setVisible] = createSignal(false);
    const [matches, setMatches] = createSignal<string[]>([]);
    const [currentIndex, setCurrentIndex] = createSignal(-1);

    const matchCount = createMemo(() => matches().length);

    const highlightId = createMemo<string | null>(() => {
        const m = matches();
        const idx = currentIndex();
        return idx >= 0 && idx < m.length ? m[idx] : null;
    });

    const performSearch = (query: string) => {
        if (!query.trim()) {
            setMatches([]);
            setCurrentIndex(-1);
            return;
        }
        const q = query.toLowerCase();
        const result: string[] = [];
        for (const node of opts.document()) {
            if (nodeSearchText(node).toLowerCase().includes(q)) {
                result.push(node.id);
            }
        }
        setMatches(result);
        const newIndex = result.length > 0 ? 0 : -1;
        setCurrentIndex(newIndex);
        if (newIndex >= 0) {
            opts.jumpTo?.(result[0]);
        }
    };

    const next = () => {
        const m = matches();
        if (m.length === 0) return;
        const n = (currentIndex() + 1) % m.length;
        setCurrentIndex(n);
        opts.jumpTo?.(m[n]);
    };

    const prev = () => {
        const m = matches();
        if (m.length === 0) return;
        const p = (currentIndex() - 1 + m.length) % m.length;
        setCurrentIndex(p);
        opts.jumpTo?.(m[p]);
    };

    const close = () => {
        setVisible(false);
        setMatches([]);
        setCurrentIndex(-1);
    };

    return {
        visible,
        setVisible,
        matches,
        currentIndex,
        matchCount,
        highlightId,
        performSearch,
        next,
        prev,
        close,
    };
}
