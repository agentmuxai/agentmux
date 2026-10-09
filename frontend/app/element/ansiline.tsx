// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, JSX } from "solid-js";

const ANSI_TAILWIND_MAP = {
    // Reset and modifiers
    0: "reset", // special: clear state
    1: "font-bold",
    2: "opacity-75",
    3: "italic",
    4: "underline",
    8: "invisible",
    9: "line-through",

    // Foreground standard colors
    30: "text-ansi-black",
    31: "text-ansi-red",
    32: "text-ansi-green",
    33: "text-ansi-yellow",
    34: "text-ansi-blue",
    35: "text-ansi-magenta",
    36: "text-ansi-cyan",
    37: "text-ansi-white",

    // Foreground bright colors
    90: "text-ansi-brightblack",
    91: "text-ansi-brightred",
    92: "text-ansi-brightgreen",
    93: "text-ansi-brightyellow",
    94: "text-ansi-brightblue",
    95: "text-ansi-brightmagenta",
    96: "text-ansi-brightcyan",
    97: "text-ansi-brightwhite",

    // Background standard colors
    40: "bg-ansi-black",
    41: "bg-ansi-red",
    42: "bg-ansi-green",
    43: "bg-ansi-yellow",
    44: "bg-ansi-blue",
    45: "bg-ansi-magenta",
    46: "bg-ansi-cyan",
    47: "bg-ansi-white",

    // Background bright colors
    100: "bg-ansi-brightblack",
    101: "bg-ansi-brightred",
    102: "bg-ansi-brightgreen",
    103: "bg-ansi-brightyellow",
    104: "bg-ansi-brightblue",
    105: "bg-ansi-brightmagenta",
    106: "bg-ansi-brightcyan",
    107: "bg-ansi-brightwhite",
};

export type AnsiState = InternalStateType;

type InternalStateType = {
    modifiers: Set<string>;
    textColor: string | null;
    bgColor: string | null;
    reverse: boolean;
};

export type AnsiSegment = SegmentType;

type SegmentType = {
    text: string;
    classes: string;
};

export const makeInitialState: () => InternalStateType = () => ({
    modifiers: new Set<string>(),
    textColor: null,
    bgColor: null,
    reverse: false,
});

export const updateStateWithCodes = (state: InternalStateType, codes: number[]) => {
    for (let i = 0; i < codes.length; i++) {
        const code = codes[i];
        if (code === 0) {
            // Reset state
            state.modifiers.clear();
            state.textColor = null;
            state.bgColor = null;
            state.reverse = false;
            continue;
        }
        // Extended colours (38/48;5;n and 38/48;2;r;g;b): no class for them, but
        // their parameters must be skipped, not read as codes ("2" is faint).
        if (code === 38 || code === 48) {
            i += codes[i + 1] === 5 ? 2 : codes[i + 1] === 2 ? 4 : 0;
            continue;
        }
        // Selective resets.
        if (code === 22) {
            state.modifiers.delete("font-bold");
            state.modifiers.delete("opacity-75");
            continue;
        }
        const off: Record<number, string> = { 23: "italic", 24: "underline", 28: "invisible", 29: "line-through" };
        if (off[code]) {
            state.modifiers.delete(off[code]);
            continue;
        }
        if (code === 27) {
            state.reverse = false;
            continue;
        }
        if (code === 39) {
            state.textColor = null;
            continue;
        }
        if (code === 49) {
            state.bgColor = null;
            continue;
        }
        // Instead of swapping immediately, we set a flag
        if (code === 7) {
            state.reverse = true;
            continue;
        }
        const tailwindClass = (ANSI_TAILWIND_MAP as any)[code];
        if (tailwindClass && tailwindClass !== "reset") {
            if (tailwindClass.startsWith("text-")) {
                state.textColor = tailwindClass;
            } else if (tailwindClass.startsWith("bg-")) {
                state.bgColor = tailwindClass;
            } else {
                state.modifiers.add(tailwindClass);
            }
        }
    }
    return state;
};

export const stateToClasses = (state: InternalStateType) => {
    const classes: string[] = [];
    classes.push(...Array.from(state.modifiers));

    // Apply reverse: swap text and background colors if flag is set.
    let textColor = state.textColor;
    let bgColor = state.bgColor;
    if (state.reverse) {
        [textColor, bgColor] = [bgColor, textColor];
    }
    if (textColor) classes.push(textColor);
    if (bgColor) classes.push(bgColor);

    return classes.join(" ");
};

const ansiRegex = /\x1b\[([0-9;]+)m/g;

const AnsiLine = ({ line }: { line: string }): JSX.Element => {
    const segments: SegmentType[] = [];
    let lastIndex = 0;
    let currentState = makeInitialState();

    // Reset regex lastIndex
    ansiRegex.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = ansiRegex.exec(line)) !== null) {
        if (match.index > lastIndex) {
            segments.push({
                text: line.substring(lastIndex, match.index),
                classes: stateToClasses(currentState),
            });
        }
        const codes = match[1].split(";").map(Number);
        updateStateWithCodes(currentState, codes);
        lastIndex = ansiRegex.lastIndex;
    }

    if (lastIndex < line.length) {
        segments.push({
            text: line.substring(lastIndex),
            classes: stateToClasses(currentState),
        });
    }

    return (
        <div>
            <For each={segments}>{(seg) => <span class={seg.classes}>{seg.text}</span>}</For>
        </div>
    );
};

export default AnsiLine;
