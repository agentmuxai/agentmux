// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { findMatch, patternFor, type SearchCall, type SearchStep } from "./hex-find";

describe("patternFor", () => {
    it("takes hex with ?? for any byte, spaced or not, in either case", () => {
        expect(patternFor("hex", "de ad ?? EF")).toEqual({ hex: "DE AD ?? EF", length: 4 });
        expect(patternFor("hex", "DEAD??EF")).toEqual({ hex: "DE AD ?? EF", length: 4 });
        expect(patternFor("hex", "  ")).toBeNull();
    });

    it("says what is wrong with hex it cannot take", () => {
        expect(patternFor("hex", "DEA")).toEqual({ error: "Whole bytes: two hex digits each, or ??" });
        expect(patternFor("hex", "GG")).toEqual({ error: "Hex digits only, ?? for any byte: DE AD ?? EF" });
        expect(patternFor("hex", "A? ??")).toEqual({ error: "A wildcard is a whole byte: ??" });
        expect(patternFor("hex", "?? ??")).toEqual({ error: "A pattern of only ?? matches everywhere" });
        expect(patternFor("hex", "AA".repeat(257))).toEqual({ error: "At most 256 bytes" });
    });

    it("turns text into its UTF-8 or UTF-16LE bytes", () => {
        expect(patternFor("utf-8", "Aé")).toEqual({ hex: "41 C3 A9", length: 3 });
        expect(patternFor("utf-16", "Aé")).toEqual({ hex: "41 00 E9 00", length: 4 });
        expect(patternFor("utf-8", "")).toBeNull();
        expect(patternFor("utf-8", "x".repeat(257))).toEqual({ error: "At most 256 bytes" });
    });
});

/** srv's search over `data`, a window of `step` starts a call (so a search takes several calls). */
function fakeSearch(data: number[], step = 8): { call: SearchCall; calls: number[] } {
    const calls: number[] = [];
    const call: SearchCall = async (req) => {
        calls.push(req.from);
        const pat = req.pattern.split(" ").map((b) => (b === "??" ? null : parseInt(b, 16)));
        const at = (i: number) => pat.every((b, k) => b === null || data[i + k] === b);
        const last = data.length - pat.length;
        const out = (found: boolean, offset: number, done: boolean, next_from: number): SearchStep => ({ found, offset, length: pat.length, done, next_from });
        if (last < 0) return out(false, 0, true, 0);
        if (!req.backward) {
            const to = Math.min(last, req.from + step - 1);
            for (let i = req.from; i <= to; i++) if (at(i)) return out(true, i, false, 0);
            return to >= last ? out(false, 0, true, data.length) : out(false, 0, false, to + 1);
        }
        const hi = Math.min(req.from, last);
        const lo = Math.max(0, hi - step + 1);
        for (let i = hi; i >= lo; i--) if (at(i)) return out(true, i, false, 0);
        return lo === 0 ? out(false, 0, true, 0) : out(false, 0, false, lo - 1);
    };
    return { call, calls };
}

const base = { path: "C:/x.bin", cancelled: () => false };

describe("findMatch", () => {
    const data = Array.from({ length: 100 }, () => 0);
    data[10] = 0xaa;
    data[70] = 0xaa;

    it("goes on call after call to a match far away", async () => {
        const { call, calls } = fakeSearch(data);
        const r = await findMatch({ ...base, call, hex: "AA", size: 100, cursor: 10, backward: false });
        expect(r).toEqual({ found: true, offset: 70, length: 1, wrapped: false });
        expect(calls.length).toBeGreaterThan(2);
    });

    it("finds the one before the cursor going backward", async () => {
        const { call } = fakeSearch(data);
        expect(await findMatch({ ...base, call, hex: "AA", size: 100, cursor: 70, backward: true })).toEqual({ found: true, offset: 10, length: 1, wrapped: false });
    });

    it("goes round to the other end, and says so", async () => {
        const { call } = fakeSearch(data);
        expect(await findMatch({ ...base, call, hex: "AA", size: 100, cursor: 70, backward: false })).toEqual({ found: true, offset: 10, length: 1, wrapped: true });
        expect(await findMatch({ ...base, call, hex: "AA", size: 100, cursor: 10, backward: true })).toEqual({ found: true, offset: 70, length: 1, wrapped: true });
    });

    it("finds the only match again after going all the way round, and says none when there is none", async () => {
        const one = Array.from({ length: 30 }, () => 0);
        one[5] = 0xbb;
        expect(await findMatch({ ...base, call: fakeSearch(one).call, hex: "BB", size: 30, cursor: 5, backward: false })).toEqual({ found: true, offset: 5, length: 1, wrapped: true });
        expect(await findMatch({ ...base, call: fakeSearch(one).call, hex: "CC", size: 30, cursor: 5, backward: false })).toEqual({ found: false });
    });

    it("starts at the other end when the cursor is on the last byte (the first, going backward)", async () => {
        const { call, calls } = fakeSearch(data);
        expect(await findMatch({ ...base, call, hex: "AA", size: 100, cursor: 99, backward: false })).toEqual({ found: true, offset: 10, length: 1, wrapped: true });
        expect(calls[0]).toBe(0);
        const back = fakeSearch(data);
        expect(await findMatch({ ...base, call: back.call, hex: "AA", size: 100, cursor: 0, backward: true })).toEqual({ found: true, offset: 70, length: 1, wrapped: true });
        expect(back.calls[0]).toBe(99);
    });

    it("stops, with nothing, when it is cancelled", async () => {
        let stop = false;
        const { call } = fakeSearch(data);
        const r = await findMatch({
            ...base,
            call: async (req) => {
                stop = true;
                return call(req);
            },
            cancelled: () => stop,
            hex: "AA",
            size: 100,
            cursor: 10,
            backward: false,
        });
        expect(r).toBeNull();
    });
});
