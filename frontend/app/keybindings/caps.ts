// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A shortcut label as key caps. `formatKey` (keys.ts) writes "Ctrl+Shift+P" elsewhere and "⇧⌘P" on macOS, and joins
// the steps of a chord with " then ". A row that draws one cap per key reads the label back into steps and keys, so
// the text stays the one source and the caps never disagree with it.

const MAC_MODIFIERS = new Set(["⌃", "⌥", "⇧", "⌘"]);

/** The keys of one step of a label: "Ctrl+Shift+P" is Ctrl, Shift, P; "Ctrl++" is Ctrl and +; "⇧⌘P" is ⇧, ⌘, P. */
function capsOfStep(step: string): string[] {
    const chars = Array.from(step);
    if (chars.length > 0 && MAC_MODIFIERS.has(chars[0])) {
        const caps: string[] = [];
        let i = 0;
        while (i < chars.length && MAC_MODIFIERS.has(chars[i])) caps.push(chars[i++]);
        const rest = chars.slice(i).join("");
        if (rest) caps.push(rest);
        return caps;
    }
    // The plus key is the one key whose own name is the separator: a trailing "+" after a "+" is the key.
    if (step.endsWith("++")) return [...capsOfStep(step.slice(0, -2)), "+"];
    if (step === "+") return ["+"];
    return step.split("+").filter((c) => c !== "");
}

/** A shortcut label as steps of key caps: `[["Ctrl", "Shift", "P"]]`, or two steps for a chord. "" gives none. */
export function keyCaps(label: string): string[][] {
    if (!label.trim()) return [];
    return label
        .split(" then ")
        .map((step) => capsOfStep(step.trim()))
        .filter((caps) => caps.length > 0);
}
