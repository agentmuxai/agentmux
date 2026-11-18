// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { SearchCall } from "../hex/hex-find";
import { findAligned, hitOrdinal, largePatternFor, splitHits } from "./large-find";

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

/** srv's byte search over `data`, finding `needle` anywhere (as filesearchbytes does, misaligned or not). */
function searchOver(data: number[]): SearchCall {
    return async (req) => {
        const pat = req.pattern.split(" ").map((b) => parseInt(b, 16));
        const at = (i: number) => pat.every((b, k) => data[i + k] === b);
        const last = data.length - pat.length;
        const range = req.backward
            ? Array.from({ length: Math.min(req.from, last) + 1 }, (_, i) => Math.min(req.from, last) - i)
            : Array.from({ length: Math.max(0, last - req.from + 1) }, (_, i) => req.from + i);
        const hit = range.find(at);
        return { found: hit != null, offset: hit ?? 0, length: pat.length, done: hit == null, next_from: 0 };
    };
}

const common = { path: "C:/x.txt", cancelled: () => false };

describe("findAligned", () => {
    // UTF-16LE "ab" = 61 00 62 00; the bytes 00 62 also appear at an odd offset.
    const data = [0x61, 0x00, 0x62, 0x00, 0x00, 0x62, 0x00, 0x00];

    it("skips a match that starts between two code units", async () => {
        const call = searchOver(data);
        // "b" is 62 00: at offset 2 (aligned) and 5 (not).
        const first = await findAligned({ ...common, call, hex: "62 00", size: data.length, cursor: -1, backward: false, align: 2 });
        expect(first).toEqual({ found: true, offset: 2, length: 2, wrapped: false });
        const second = await findAligned({ ...common, call, hex: "62 00", size: data.length, cursor: 2, backward: false, align: 2 });
        // Offset 5 is misaligned: the search goes round to the aligned one it began at.
        expect(second).toMatchObject({ found: true, offset: 2, wrapped: true });
    });

    it("ends when every match is misaligned", async () => {
        const odd = [0x00, 0x62, 0x00, 0x00];
        const r = await findAligned({ ...common, call: searchOver(odd), hex: "62 00", size: odd.length, cursor: -1, backward: false, align: 2 });
        expect(r).toEqual({ found: false });
    });

    it("takes every match when the encoding is single-byte", async () => {
        const r = await findAligned({ ...common, call: searchOver(data), hex: "62 00", size: data.length, cursor: 2, backward: false, align: 1 });
        expect(r).toMatchObject({ found: true, offset: 5 });
    });
});
