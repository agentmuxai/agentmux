// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's find: what a typed text is as bytes in the file's
 * encoding, and the search over srv (`filesearchbytes`, the hex viewer's own),
 * whose matches are placed in lines by `fileindexlines`. Exact case: the search
 * is of bytes. docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
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

export interface TextPart {
    text: string;
    hit: boolean;
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
export function hitOrdinal(line: string, needle: string, byteInLine: number, encoding: string | undefined): number {
    if (!needle) return -1;
    let ordinal = 0;
    let at = 0; // the char the bytes below are counted to
    let bytes = 0; // the bytes of line[0..at]
    for (;;) {
        const i = line.indexOf(needle, at);
        if (i < 0) return -1;
        bytes += bytesIn(line.slice(at, i), encoding);
        at = i;
        if (bytes >= byteInLine) return ordinal;
        ordinal++;
        bytes += bytesIn(needle, encoding);
        at = i + needle.length;
    }
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
