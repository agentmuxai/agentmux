// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Address bar suggestions for a browser pane: Chrome's own omnibox engine,
 * asked through the host (`browser_pane_autocomplete`, crates/cef
 * browser_panes/autocomplete.rs). History and typed URLs with inline
 * completion, "search for what you typed", and the default search engine's
 * suggestions, from the pane's own profile.
 *
 * At most one query is in flight: while it runs, newer text replaces whatever
 * is waiting, and goes when it returns. A result for text that has since
 * changed is never shown. A host without the API answers `supported: false`,
 * and the address bar keeps its plain behavior.
 *
 * Spike: docs/analysis/ANALYSIS_BROWSER_ADDRESS_BAR_AUTOCOMPLETE_SPIKE_2026_10_12.md.
 */

import { createSignal, type Accessor } from "solid-js";

export type AutocompleteQuery = (text: string) => Promise<BrowserAutocompleteResult | undefined>;

export interface AddressAutocomplete {
    /** The current suggestions, best first; empty when closed. */
    matches: Accessor<BrowserAutocompleteMatch[]>;
    /** The text `matches` answer, so a stale list is never acted on. */
    matchesFor: Accessor<string>;
    /** Highlighted row, or -1. */
    selected: Accessor<number>;
    /** False once the host has said it has no autocomplete. */
    supported: Accessor<boolean>;
    /** New text in the address bar. */
    update(text: string): void;
    /** Move the highlight by `delta` rows (wraps; -1 means none). */
    move(delta: number): void;
    select(index: number): void;
    /** Close the list and drop any query still waiting. */
    close(): void;
    /** The match Enter should open for `text`: the highlighted one, else the
     *  engine's default match. Undefined when the list doesn't answer `text`. */
    chosen(text: string): BrowserAutocompleteMatch | undefined;
}

export function createAddressAutocomplete(query: AutocompleteQuery): AddressAutocomplete {
    const [matches, setMatches] = createSignal<BrowserAutocompleteMatch[]>([]);
    const [matchesFor, setMatchesFor] = createSignal("");
    const [selected, setSelected] = createSignal(-1);
    const [supported, setSupported] = createSignal(true);

    let current = ""; // the newest text asked for
    let inFlight = false;
    let waiting: string | null = null;

    const run = (text: string) => {
        inFlight = true;
        query(text)
            .then((result) => {
                if (!result) return;
                if (!result.supported) {
                    setSupported(false);
                    return;
                }
                // Only the answer for the newest text is shown.
                if (text !== current || waiting !== null) return;
                setMatches(result.matches);
                setMatchesFor(text);
                setSelected(-1);
            })
            .catch(() => {})
            .finally(() => {
                inFlight = false;
                if (waiting !== null) {
                    const next = waiting;
                    waiting = null;
                    run(next);
                }
            });
    };

    const close = () => {
        current = "";
        waiting = null;
        setMatches([]);
        setMatchesFor("");
        setSelected(-1);
    };

    return {
        matches,
        matchesFor,
        selected,
        supported,
        update(text) {
            if (!supported()) return;
            if (!text.trim()) {
                close();
                return;
            }
            current = text;
            if (inFlight) waiting = text;
            else run(text);
        },
        move(delta) {
            const n = matches().length;
            if (n === 0) return;
            // -1 (no highlight) sits between the last row and the first.
            const pos = (selected() + 1 + delta + (n + 1)) % (n + 1);
            setSelected(pos - 1);
        },
        select(index) {
            setSelected(index >= 0 && index < matches().length ? index : -1);
        },
        close,
        chosen(text) {
            if (matchesFor() !== text) return undefined;
            const list = matches();
            if (selected() >= 0) return list[selected()];
            return list[0]?.allowed_to_be_default ? list[0] : undefined;
        },
    };
}

/** The text to show after what was typed, when the engine's default match
 *  completes it inline (Chrome's "git" -> "git[hub.com]"). Empty while the user
 *  is deleting, and when the list answers other text. */
export function inlineCompletion(typed: string, result: { matches: BrowserAutocompleteMatch[]; matchesFor: string }, deleting: boolean): string {
    if (deleting || result.matchesFor !== typed) return "";
    const top = result.matches[0];
    if (!top?.allowed_to_be_default || !top.inline_autocompletion) return "";
    return top.inline_autocompletion;
}
