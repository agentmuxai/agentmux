// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's find: what a typed text is as bytes in the file's
 * encoding, and the search over srv (`filesearchbytes`, the hex viewer's own),
 * whose matches are placed in lines by `fileindexlines`. An exact text is a search
 * of bytes; ignoring case or a regular expression is a search of text
 * (`filesearchtext`, UTF-8 files). docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { hexOfBytes, MAX_PATTERN_BYTES } from "../hex/hex-find";

export type LargePattern = { hex: string; length: number; align: 1 | 2 } | { error: string } | null;

/**
 * The legacy encodings whose trail bytes can equal an ASCII byte (Shift_JIS 83 5C is one katakana; a search for a
 * backslash would match its 5C): a byte search finds characters that are not there. Names as srv gives them (WHATWG).
 */
const MULTIBYTE = /^(shift[_-]?jis|windows-31j|ms932|euc-jp|euc-kr|iso-2022-jp|gbk|gb2312|gb18030|big5|big5-hkscs)$/i;

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
 * A multibyte legacy encoding (Shift_JIS, GBK, Big5, EUC) is refused: an ASCII byte can sit inside one of its characters.
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
    } else if (MULTIBYTE.test(enc)) {
        return { error: `This file is in ${encoding}, which spells some characters with several bytes: it can't be searched yet` };
    } else if (Array.from(text).every((c) => c.charCodeAt(0) < 128)) {
        bytes = Array.from(text, (c) => c.charCodeAt(0));
    } else {
        return { error: `This file is in ${enc === "legacy" ? "an older encoding" : enc}: only plain ASCII text can be found` };
    }
    if (bytes.length > MAX_PATTERN_BYTES) return { error: `At most ${MAX_PATTERN_BYTES} bytes` };
    return { hex: hexOfBytes(bytes), length: bytes.length, align };
}

/** How a typed text is read: "Aa" on matches case, ".*" makes it a regular expression. */
export interface FindOptions {
    matchCase: boolean;
    regex: boolean;
}

export const EXACT: FindOptions = { matchCase: true, regex: false };

/** The longest text srv takes for a text or regex search, in characters. */
export const MAX_QUERY_CHARS = 1000;

/** What to ask srv for: the bytes of an exact text (`filesearchbytes`), or the text itself (`filesearchtext`, UTF-8 files). */
export type LargeSearch =
    | { kind: "bytes"; hex: string; length: number; align: 1 | 2 }
    | { kind: "text"; query: string; regex: boolean; ignoreCase: boolean }
    | { error: string }
    | null;

/**
 * What the find strip's text asks for. An exact, literal text is a search of bytes, in any encoding largePatternFor
 * takes; ignoring case or a regular expression is a search of text, which srv does for UTF-8 only.
 */
export function largeSearchFor(encoding: string | undefined, text: string, o: FindOptions): LargeSearch {
    if (!text) return null;
    if (o.matchCase && !o.regex) {
        const p = largePatternFor(encoding, text);
        if (!p || "error" in p) return p;
        return { kind: "bytes", ...p };
    }
    const enc = (encoding ?? "utf-8").toLowerCase();
    if (enc !== "utf-8" && enc !== "utf8") {
        const what = o.regex ? "A regular expression" : "A search that ignores case";
        return { error: `${what} can only search a UTF-8 file; this one is in ${enc === "legacy" ? "an older encoding" : enc}` };
    }
    if (Array.from(text).length > MAX_QUERY_CHARS) return { error: `At most ${MAX_QUERY_CHARS} characters` };
    return { kind: "text", query: text, regex: o.regex, ignoreCase: !o.matchCase };
}

/** Marks the text found in a line: the literal text, or a global expression; "" when nothing can be marked. */
export type LineMatcher = string | RegExp;

function escapeRegex(text: string): string {
    return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * What marks a found text in a line. An exact text is itself. Otherwise an expression of the same meaning in
 * JavaScript, which can differ from srv's in a corner (then a line may show fewer marks, never a wrong place found).
 */
export function matcherFor(text: string, o: FindOptions): LineMatcher {
    if (!text) return "";
    if (o.matchCase && !o.regex) return text;
    const source = o.regex ? text : escapeRegex(text);
    const flags = o.matchCase ? "gm" : "gim";
    for (const extra of ["u", ""]) {
        try {
            return new RegExp(source, flags + extra);
        } catch {
            // Try without the unicode flag, then give up marking.
        }
    }
    return "";
}

export interface TextPart {
    text: string;
    hit: boolean;
}

/** Where `matcher` matches in `line`: the start in characters and the text, left to right, none empty. */
function occurrences(line: string, matcher: LineMatcher): Array<{ index: number; text: string }> {
    const out: Array<{ index: number; text: string }> = [];
    if (!matcher) return out;
    if (typeof matcher === "string") {
        for (let at = 0; ; ) {
            const i = line.indexOf(matcher, at);
            if (i < 0) break;
            out.push({ index: i, text: matcher });
            at = i + matcher.length;
        }
        return out;
    }
    for (const m of line.matchAll(matcher)) {
        if (m[0]) out.push({ index: m.index ?? 0, text: m[0] });
    }
    return out;
}

/** How many bytes `text` takes in a file of `encoding`. A single-byte encoding other than UTF-8 is a byte a character. */
function bytesIn(text: string, encoding: string | undefined): number {
    const enc = (encoding ?? "utf-8").toLowerCase();
    if (enc === "utf-16le" || enc === "utf-16be") return text.length * 2;
    if (enc === "utf-8" || enc === "utf8") return new TextEncoder().encode(text).length;
    return text.length;
}

/**
 * Which of the places `needle` occurs in `line` (0 for the first) is the match that starts `byteInLine` bytes into the
 * line, in a file of `encoding`: the one the search found, among several on the line. The first that starts at or past
 * that byte when none starts exactly there (a byte-order mark or an odd character shifts the count); -1 when there is
 * none in the text drawn (a long line is cut short), or nothing to find.
 */
export function hitOrdinal(line: string, needle: LineMatcher, byteInLine: number, encoding: string | undefined): number {
    let ordinal = 0;
    let at = 0; // the char the bytes below are counted to
    let bytes = 0; // the bytes of line[0..at]
    for (const m of occurrences(line, needle)) {
        bytes += bytesIn(line.slice(at, m.index), encoding);
        if (bytes >= byteInLine) return ordinal;
        ordinal++;
        bytes += bytesIn(m.text, encoding);
        at = m.index + m.text.length;
    }
    return -1;
}

/** `line` cut at each place `needle` matches in it, for marking them. */
export function splitHits(line: string, needle: LineMatcher): TextPart[] {
    const parts: TextPart[] = [];
    let at = 0;
    for (const m of occurrences(line, needle)) {
        if (m.index > at) parts.push({ text: line.slice(at, m.index), hit: false });
        parts.push({ text: m.text, hit: true });
        at = m.index + m.text.length;
    }
    if (at < line.length || !parts.length) parts.push({ text: line.slice(at), hit: false });
    return parts;
}
