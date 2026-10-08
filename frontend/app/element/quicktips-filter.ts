// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the Help pane's filter adds to the shared `matchesEveryWord`
// (util/fuzzysearch.ts): macOS key symbols read as their names, so
// "cmd shift w" finds "⇧⌘W". SPEC_HELP_PANE_FILTER_2026_10_08.md §3.

const KEY_SYMBOL_WORDS: Record<string, string> = {
    "⌘": "cmd command",
    "⇧": "shift",
    "⌥": "opt option alt",
    "⌃": "ctrl control",
    "↑": "up arrow",
    "↓": "down arrow",
    "←": "left arrow",
    "→": "right arrow",
    "⏎": "enter return",
    "↩": "enter return",
    "⌫": "backspace delete",
    "⎋": "esc escape",
};

/** A key label plus the names of the symbols in it: "⇧⌘W" → "⇧⌘W shift cmd command". */
export function keyLabelWords(label: string): string {
    let words = label;
    for (const ch of label) {
        const named = KEY_SYMBOL_WORDS[ch];
        if (named) words += " " + named;
    }
    return words;
}
