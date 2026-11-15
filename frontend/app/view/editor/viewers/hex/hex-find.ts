// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer's find: what a typed pattern means as bytes, and the search
 * over srv (`filesearchbytes`), which reads the file in blocks and hands back
 * where it got to when a call runs long. docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

export type FindMode = "hex" | "utf-8" | "utf-16";

export const FIND_MODES: ReadonlyArray<{ value: FindMode; label: string }> = [
    { value: "hex", label: "Hex" },
    { value: "utf-8", label: "UTF-8" },
    { value: "utf-16", label: "UTF-16" },
];

/** The longest pattern srv takes, in bytes. */
export const MAX_PATTERN_BYTES = 256;

export type FindPattern = { hex: string; length: number } | { error: string } | null;

function hexOfBytes(bytes: number[]): string {
    return bytes.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join(" ");
}

function utf16le(text: string): number[] {
    const out: number[] = [];
    for (let i = 0; i < text.length; i++) {
        const unit = text.charCodeAt(i);
        out.push(unit & 0xff, unit >> 8);
    }
    return out;
}

/** What `text` is as bytes in `mode`: null for nothing typed yet, an error to show, or the hex srv takes. */
export function patternFor(mode: FindMode, text: string): FindPattern {
    if (mode === "hex") {
        const digits = text.replace(/\s+/g, "");
        if (!digits) return null;
        if (!/^(?:[0-9a-fA-F?])*$/.test(digits)) return { error: "Hex digits only, ?? for any byte: DE AD ?? EF" };
        if (digits.length % 2) return { error: "Whole bytes: two hex digits each, or ??" };
        const bytes: string[] = digits.match(/../g) ?? [];
        if (bytes.some((b) => b.includes("?") && b !== "??")) return { error: "A wildcard is a whole byte: ??" };
        if (bytes.length > MAX_PATTERN_BYTES) return { error: `At most ${MAX_PATTERN_BYTES} bytes` };
        if (bytes.every((b) => b === "??")) return { error: "A pattern of only ?? matches everywhere" };
        return { hex: bytes.map((b) => b.toUpperCase()).join(" "), length: bytes.length };
    }
    if (!text) return null;
    const bytes = mode === "utf-8" ? Array.from(new TextEncoder().encode(text)) : utf16le(text);
    if (bytes.length > MAX_PATTERN_BYTES) return { error: `At most ${MAX_PATTERN_BYTES} bytes` };
    return { hex: hexOfBytes(bytes), length: bytes.length };
}

export interface SearchStep {
    found: boolean;
    offset: number;
    length: number;
    done: boolean;
    next_from: number;
}

export type SearchCall = (req: { path: string; pattern: string; from: number; backward: boolean }) => Promise<SearchStep>;

export type FindResult = { found: true; offset: number; length: number; wrapped: boolean } | { found: false };

/** A search that goes on, call after call, until a match, the end of the file, or `cancelled`. */
async function sweep(call: SearchCall, path: string, pattern: string, from: number, backward: boolean, cancelled: () => boolean): Promise<SearchStep | null> {
    let at = from;
    for (;;) {
        const step = await call({ path, pattern, from: at, backward });
        if (cancelled()) return null;
        if (step.found || step.done) return step;
        at = step.next_from;
    }
}

/**
 * The next match after `cursor` (the one before it, going backward), the
 * search going round to the other end of the file once. null when `cancelled`
 * said stop (a newer search began, the tab closed).
 */
export async function findMatch(o: {
    call: SearchCall;
    path: string;
    hex: string;
    size: number;
    /** Where the cursor is: the search starts just past it. */
    cursor: number;
    backward: boolean;
    cancelled: () => boolean;
}): Promise<FindResult | null> {
    const first = o.backward ? o.cursor - 1 : o.cursor + 1;
    const edge = o.backward ? first < 0 : first >= o.size;
    if (!edge) {
        const step = await sweep(o.call, o.path, o.hex, first, o.backward, o.cancelled);
        if (!step) return null;
        if (step.found) return { found: true, offset: step.offset, length: step.length, wrapped: false };
    }
    // Round to the other end.
    const step = await sweep(o.call, o.path, o.hex, o.backward ? Math.max(0, o.size - 1) : 0, o.backward, o.cancelled);
    if (!step) return null;
    return step.found ? { found: true, offset: step.offset, length: step.length, wrapped: true } : { found: false };
}
