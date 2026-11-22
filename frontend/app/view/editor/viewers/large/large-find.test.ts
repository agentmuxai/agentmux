// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { SearchCall } from "../hex/hex-find";
import { findMatch } from "../hex/hex-find";
import { hitOrdinal, largePatternFor, largeSearchFor, matcherFor, splitHits } from "./large-find";

describe("largePatternFor", () => {
    it("is the text's bytes in the file's encoding", () => {
        expect(largePatternFor("utf-8", "Aé")).toEqual({ hex: "41 C3 A9", length: 3, align: 1 });
        expect(largePatternFor(undefined, "A")).toEqual({ hex: "41", length: 1, align: 1 });
        expect(largePatternFor("utf-16le", "Aé")).toEqual({ hex: "41 00 E9 00", length: 4, align: 2 });
        expect(largePatternFor("utf-16be", "Aé")).toEqual({ hex: "00 41 00 E9", length: 4, align: 2 });
        expect(largePatternFor("utf-8", "")).toBeNull();
    });

    it("takes plain ASCII in a single-byte encoding and refuses the rest, saying why", () => {
        expect(largePatternFor("legacy", "ERROR")).toEqual({ hex: "45 52 52 4F 52", length: 5, align: 1 });
        expect(largePatternFor("windows-1251", "x")).toMatchObject({ hex: "78" });
        expect(largePatternFor("legacy", "é")).toEqual({ error: "This file is in an older encoding: only plain ASCII text can be found" });
        expect(largePatternFor("windows-1251", "é")).toEqual({ error: "This file is in windows-1251: only plain ASCII text can be found" });
    });

    it("refuses a multibyte legacy encoding, where an ASCII byte can be half of a character", () => {
        for (const enc of ["Shift_JIS", "GBK", "Big5", "EUC-JP", "EUC-KR", "gb18030", "ISO-2022-JP"]) {
            expect(largePatternFor(enc, "\\")).toEqual({ error: `This file is in ${enc}, which spells some characters with several bytes: it can't be searched yet` });
        }
        expect(largePatternFor("windows-1252", "\\")).toMatchObject({ hex: "5C" });
    });

    it("takes at most as many bytes as srv does", () => {
        expect(largePatternFor("utf-8", "x".repeat(256))).toMatchObject({ length: 256 });
        expect(largePatternFor("utf-8", "x".repeat(257))).toEqual({ error: "At most 256 bytes" });
        expect(largePatternFor("utf-16le", "x".repeat(129))).toEqual({ error: "At most 256 bytes" });
    });
});

describe("hitOrdinal", () => {
    it("names which of a line's matches starts at a byte of the line", () => {
        // "a err b err c err": the matches start at bytes 2, 8 and 14.
        const line = "a err b err c err";
        expect(hitOrdinal(line, "err", 2, "utf-8")).toBe(0);
        expect(hitOrdinal(line, "err", 8, "utf-8")).toBe(1);
        expect(hitOrdinal(line, "err", 14, "utf-8")).toBe(2);
    });

    it("counts the bytes of the file's encoding, not the characters", () => {
        // "é" is two bytes in UTF-8 and every character two bytes in UTF-16.
        expect(hitOrdinal("é err é err", "err", 3, "utf-8")).toBe(0);
        expect(hitOrdinal("é err é err", "err", 10, "utf-8")).toBe(1);
        expect(hitOrdinal("a err b err", "err", 4, "utf-16le")).toBe(0);
        expect(hitOrdinal("a err b err", "err", 16, "utf-16le")).toBe(1);
    });

    it("takes the first match at or past the byte when none starts there, and none when the line is cut before it", () => {
        expect(hitOrdinal("a err b err", "err", 5, "utf-8")).toBe(1);
        expect(hitOrdinal("a err b err", "err", 99, "utf-8")).toBe(-1);
        expect(hitOrdinal("abc", "err", 0, "utf-8")).toBe(-1);
        expect(hitOrdinal("abc", "", 0, "utf-8")).toBe(-1);
    });
});

describe("splitHits", () => {
    it("cuts a line at each place the text occurs", () => {
        expect(splitHits("a err b err", "err")).toEqual([
            { text: "a ", hit: false },
            { text: "err", hit: true },
            { text: " b ", hit: false },
            { text: "err", hit: true },
        ]);
        expect(splitHits("err", "err")).toEqual([{ text: "err", hit: true }]);
    });

    it("leaves a line alone when nothing is to be marked or nothing matches", () => {
        expect(splitHits("abc", "")).toEqual([{ text: "abc", hit: false }]);
        expect(splitHits("abc", "x")).toEqual([{ text: "abc", hit: false }]);
        expect(splitHits("", "x")).toEqual([{ text: "", hit: false }]);
    });
});

describe("largeSearchFor", () => {
    const exact = { matchCase: true, regex: false };
    it("keeps an exact literal text a search of bytes, as before", () => {
        expect(largeSearchFor("utf-8", "Aé", exact)).toEqual({ kind: "bytes", hex: "41 C3 A9", length: 3, align: 1 });
        expect(largeSearchFor("utf-16le", "A", exact)).toMatchObject({ kind: "bytes", align: 2 });
        expect(largeSearchFor("utf-8", "", exact)).toBeNull();
        expect(largeSearchFor("Shift_JIS", "\\", exact)).toMatchObject({ error: expect.stringContaining("several bytes") });
    });

    it("asks srv for text when case is ignored or the text is an expression", () => {
        expect(largeSearchFor("utf-8", "Err", { matchCase: false, regex: false })).toEqual({ kind: "text", query: "Err", regex: false, ignoreCase: true });
        expect(largeSearchFor(undefined, "a+", { matchCase: true, regex: true })).toEqual({ kind: "text", query: "a+", regex: true, ignoreCase: false });
        expect(largeSearchFor("utf-8", "a+", { matchCase: false, regex: true })).toMatchObject({ regex: true, ignoreCase: true });
    });

    it("says why a file in another encoding can't be searched that way", () => {
        expect(largeSearchFor("utf-16le", "a", { matchCase: false, regex: false })).toEqual({
            error: "A search that ignores case can only search a UTF-8 file; this one is in utf-16le",
        });
        expect(largeSearchFor("legacy", "a", { matchCase: true, regex: true })).toEqual({
            error: "A regular expression can only search a UTF-8 file; this one is in an older encoding",
        });
    });

    it("takes at most as many characters as srv does", () => {
        expect(largeSearchFor("utf-8", "x".repeat(1000), { matchCase: false, regex: false })).toMatchObject({ kind: "text" });
        expect(largeSearchFor("utf-8", "x".repeat(1001), { matchCase: false, regex: false })).toEqual({ error: "At most 1000 characters" });
    });
});

describe("matcherFor", () => {
    it("is the text itself for an exact search, so its marks are as before", () => {
        expect(matcherFor("a.c", { matchCase: true, regex: false })).toBe("a.c");
        expect(matcherFor("", { matchCase: false, regex: true })).toBe("");
    });

    it("marks a text in either case, its metacharacters meaning themselves", () => {
        const m = matcherFor("a.c", { matchCase: false, regex: false });
        expect(splitHits("A.C abc a.c", m).filter((p) => p.hit).map((p) => p.text)).toEqual(["A.C", "a.c"]);
    });

    it("marks what an expression matches, skipping empty matches", () => {
        const m = matcherFor("err\\d*", { matchCase: true, regex: true });
        expect(splitHits("err1 ERR2 err", m).filter((p) => p.hit).map((p) => p.text)).toEqual(["err1", "err"]);
        expect(splitHits("abc", matcherFor("x*", { matchCase: true, regex: true }))).toEqual([{ text: "abc", hit: false }]);
    });

    it("marks nothing for an expression JavaScript can't read", () => {
        expect(matcherFor("(", { matchCase: true, regex: true })).toBe("");
    });

    it("names the ordinal of a match found by an expression, counting the file's bytes", () => {
        const m = matcherFor("é|err", { matchCase: true, regex: true });
        // "é err": é at byte 0 (2 bytes), "err" at byte 3.
        expect(hitOrdinal("é err", m, 0, "utf-8")).toBe(0);
        expect(hitOrdinal("é err", m, 3, "utf-8")).toBe(1);
    });
});

/** srv's byte search over `data`, as filesearchbytes does: a match must start at a multiple of `req.align`. */
function searchOver(data: number[], calls: unknown[] = []): SearchCall {
    return async (req) => {
        calls.push(req);
        const pat = req.pattern.split(" ").map((b) => parseInt(b, 16));
        const at = (i: number) => i % (req.align ?? 1) === 0 && pat.every((b, k) => data[i + k] === b);
        const last = data.length - pat.length;
        const range = req.backward
            ? Array.from({ length: Math.min(req.from, last) + 1 }, (_, i) => Math.min(req.from, last) - i)
            : Array.from({ length: Math.max(0, last - req.from + 1) }, (_, i) => req.from + i);
        const hit = range.find(at);
        return { found: hit != null, offset: hit ?? 0, length: pat.length, done: hit == null, next_from: 0 };
    };
}

const common = { path: "C:/x.txt", cancelled: () => false };

describe("a UTF-16 search over srv", () => {
    // UTF-16LE "ab" = 61 00 62 00; the bytes 62 00 also appear at an odd offset (5).
    const data = [0x61, 0x00, 0x62, 0x00, 0x00, 0x62, 0x00, 0x00];

    it("asks srv for matches on a two-byte boundary, one call per sweep however many are misaligned", async () => {
        const calls: unknown[] = [];
        const call = searchOver(data, calls);
        const first = await findMatch({ ...common, call, hex: "62 00", size: data.length, cursor: -1, backward: false, align: 2 });
        expect(first).toEqual({ found: true, offset: 2, length: 2, wrapped: false });
        expect(calls).toEqual([{ path: "C:/x.txt", pattern: "62 00", from: 0, backward: false, align: 2 }]);
        // Offset 5 is misaligned and srv never reports it: the search goes round to the aligned one it began at.
        const second = await findMatch({ ...common, call, hex: "62 00", size: data.length, cursor: 2, backward: false, align: 2 });
        expect(second).toMatchObject({ found: true, offset: 2, wrapped: true });
    });

    it("ends when every match is misaligned", async () => {
        const odd = [0x00, 0x62, 0x00, 0x00];
        const r = await findMatch({ ...common, call: searchOver(odd), hex: "62 00", size: odd.length, cursor: -1, backward: false, align: 2 });
        expect(r).toEqual({ found: false });
    });

    it("sends no alignment for a single-byte encoding, and takes every match", async () => {
        const calls: unknown[] = [];
        const r = await findMatch({ ...common, call: searchOver(data, calls), hex: "62 00", size: data.length, cursor: 2, backward: false, align: 1 });
        expect(r).toMatchObject({ found: true, offset: 5 });
        expect(calls).toEqual([{ path: "C:/x.txt", pattern: "62 00", from: 3, backward: false }]);
    });
});
