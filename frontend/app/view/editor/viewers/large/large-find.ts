// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's find: what a typed text is as bytes in the file's
 * encoding, and the search over srv (`filesearchbytes`, the hex viewer's own),
 * whose matches are placed in lines by `fileindexlines`. Exact case: the search
 * is of bytes. docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { findMatch, hexOfBytes, MAX_PATTERN_BYTES, type FindResult } from "../hex/hex-find";

export type LargePattern = { hex: string; length: number; align: 1 | 2 } | { error: string } | null;

/** The UTF-16 bytes of `text`, little or big end first. */
function utf16(text: string, little: boolean): number[] {
    const out: number[] = [];
    for (let i = 0; i < text.length; i++) {
        const unit = text.charCodeAt(i);
        out.push(...(little ? [unit & 0xff, unit >> 8] : [unit >> 8, unit & 0xff]));
    }
    return out;
}

/**
 * What `text` is as bytes in a file of `encoding` (the probe's or srv's name for it): null for nothing typed, an error
 * to show, or the hex srv takes with the alignment a match must have (a UTF-16 match is on a two-byte boundary).
 * A single-byte encoding other than UTF-8 is searched for plain ASCII only: those bytes mean the same in all of them.
 */
export function largePatternFor(encoding: string | undefined, text: string): LargePattern {
    if (!text) return null;
    const enc = (encoding ?? "utf-8").toLowerCase();
    let bytes: number[];
    let align: 1 | 2 = 1;
    if (enc === "utf-16le" || enc === "utf-16be") {
        bytes = utf16(text, enc === "utf-16le");
        align = 2;
    } else if (enc === "utf-8" || enc === "utf8") {
        bytes = Array.from(new TextEncoder().encode(text));
    } else if (Array.from(text).every((c) => c.charCodeAt(0) < 128)) {
        bytes = Array.from(text, (c) => c.charCodeAt(0));
    } else {
        return { error: `This file is in ${enc === "legacy" ? "an older encoding" : enc}: only plain ASCII text can be found` };
    }
    if (bytes.length > MAX_PATTERN_BYTES) return { error: `At most ${MAX_PATTERN_BYTES} bytes` };
    return { hex: hexOfBytes(bytes), length: bytes.length, align };
}

/** What `findMatch` takes, less what a search over this file fills in. */
export type AlignedFind = Parameters<typeof findMatch>[0] & { align: 1 | 2 };

/**
 * `findMatch`, skipping the matches that start in the middle of a UTF-16 unit (the bytes of one character's end and
 * the next one's start can spell the text). Stops at the first such match it sees again, so it ends.
 */
export async function findAligned(o: AlignedFind): Promise<FindResult | null> {
    let cursor = o.cursor;
    let first = -1;
    for (;;) {
        const r = await findMatch({ ...o, cursor });
        if (!r || !r.found || r.offset % o.align === 0) return r;
        if (r.offset === first) return { found: false };
        if (first < 0) first = r.offset;
        cursor = r.offset;
    }
}

export interface TextPart {
    text: string;
    hit: boolean;
}

/** `line` cut at each place `needle` occurs in it, for marking them. */
export function splitHits(line: string, needle: string): TextPart[] {
    if (!needle) return [{ text: line, hit: false }];
    const parts: TextPart[] = [];
    let at = 0;
    for (;;) {
        const i = line.indexOf(needle, at);
        if (i < 0) break;
        if (i > at) parts.push({ text: line.slice(at, i), hit: false });
        parts.push({ text: needle, hit: true });
        at = i + needle.length;
    }
    if (at < line.length || !parts.length) parts.push({ text: line.slice(at), hit: false });
    return parts;
}
