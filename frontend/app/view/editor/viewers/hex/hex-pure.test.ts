// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer's pure parts: rows and scroll positions (a 4 GB file
 * included), offsets as written and as typed, where the keys move the
 * cursor, the parts cache, the data inspector and what a copy writes.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { describe, expect, it, vi } from "vitest";
import { CHUNK, ChunkCache } from "./hex-chunks";
import { asBase64, asCArray, asHex, asText, copyText } from "./hex-copy";
import { float16, inspect, uleb128, utf8Char } from "./hex-inspector";
import {
    charOf,
    contentHeight,
    firstRowAt,
    hexOf,
    isScaled,
    MAX_SCROLL_PX,
    maxFirstRow,
    moveCursor,
    offsetDigits,
    offsetText,
    parseOffset,
    ROW_H,
    rowCount,
    rowsInView,
    scrollTopFor,
} from "./hex-layout";

const GB4 = 4 * 1024 ** 3;

describe("rows and scrolling", () => {
    it("counts whole and part rows, and none for an empty file", () => {
        expect(rowCount(0, 16)).toBe(0);
        expect(rowCount(1, 16)).toBe(1);
        expect(rowCount(16, 16)).toBe(1);
        expect(rowCount(17, 16)).toBe(2);
        expect(rowCount(GB4, 16)).toBe(268_435_456);
    });

    it("scrolls a small file a row to a row's height", () => {
        const rows = 1000;
        expect(isScaled(rows)).toBe(false);
        expect(contentHeight(rows)).toBe(rows * ROW_H);
        expect(firstRowAt(0, rows, 400)).toBe(0);
        expect(firstRowAt(ROW_H * 7 + 5, rows, 400)).toBe(7);
        expect(scrollTopFor(7, rows, 400)).toBe(7 * ROW_H);
        // Never past the row that puts the last one at the bottom.
        expect(maxFirstRow(rows, 400)).toBe(rows - rowsInView(400));
        expect(firstRowAt(1e9, rows, 400)).toBe(980);
        expect(scrollTopFor(5000, rows, 400)).toBe(980 * ROW_H);
    });

    it("scrolls a 4 GB file in proportion, and reaches its first and last rows", () => {
        const rows = rowCount(GB4, 16);
        const view = 400;
        expect(isScaled(rows)).toBe(true);
        expect(contentHeight(rows)).toBe(MAX_SCROLL_PX);
        expect(firstRowAt(0, rows, view)).toBe(0);
        expect(firstRowAt(MAX_SCROLL_PX - view, rows, view)).toBe(maxFirstRow(rows, view));
        expect(scrollTopFor(maxFirstRow(rows, view), rows, view)).toBe(MAX_SCROLL_PX - view);
        // A row, to its scroll position and back, is within the rows one pixel stands for.
        const perPixel = Math.ceil(rows / (MAX_SCROLL_PX - view));
        for (const row of [1, 12_345, 134_217_728, 268_000_000]) {
            const back = firstRowAt(scrollTopFor(row, rows, view), rows, view);
            expect(Math.abs(back - row)).toBeLessThanOrEqual(perPixel);
        }
    });

    it("shows at least one row in a view with no height", () => {
        expect(rowsInView(0)).toBe(1);
        expect(firstRowAt(0, 10, 0)).toBe(0);
    });
});

describe("offsets and bytes as written", () => {
    it("writes a byte as two hex digits and as its ASCII character or a dot", () => {
        expect(hexOf(0)).toBe("00");
        expect(hexOf(0xab)).toBe("AB");
        expect(hexOf(255)).toBe("FF");
        expect(charOf(0x41)).toBe("A");
        expect(charOf(0x20)).toBe(" ");
        expect(charOf(0x7e)).toBe("~");
        expect(charOf(0x1f)).toBe(".");
        expect(charOf(0x7f)).toBe(".");
        expect(charOf(0xe9)).toBe(".");
    });

    it("gives the offset column 8 digits, more past 4 GB", () => {
        expect(offsetDigits(0)).toBe(8);
        expect(offsetDigits(GB4)).toBe(8);
        expect(offsetDigits(GB4 + 1)).toBe(9);
        expect(offsetText(0x1f, 8)).toBe("0000001F");
        expect(offsetText(GB4, 9)).toBe("100000000");
    });

    it("reads a typed offset as decimal or hex, and refuses one outside the file", () => {
        expect(parseOffset("31", 100)).toBe(31);
        expect(parseOffset(" 0x1F ", 100)).toBe(31);
        expect(parseOffset("1Fh", 100)).toBe(31);
        expect(parseOffset("1,000", 2000)).toBe(1000);
        expect(parseOffset("0", 1)).toBe(0);
        expect(parseOffset("100", 100)).toBeNull();
        expect(parseOffset("-1", 100)).toBeNull();
        expect(parseOffset("1F", 100)).toBeNull();
        expect(parseOffset("", 100)).toBeNull();
        expect(parseOffset("0x", 100)).toBeNull();
        expect(parseOffset("0", 0)).toBeNull();
    });
});

describe("where the keys move the cursor", () => {
    const move = (key: string, at: number, mod = false) => moveCursor(key, mod, at, 100, 16, 4);

    it("moves a byte, a row and a page, and stays inside the file", () => {
        expect(move("ArrowRight", 15)).toBe(16);
        expect(move("ArrowLeft", 16)).toBe(15);
        expect(move("ArrowLeft", 0)).toBe(0);
        expect(move("ArrowRight", 99)).toBe(99);
        expect(move("ArrowDown", 5)).toBe(21);
        expect(move("ArrowUp", 5)).toBe(5);
        // The last row is short: no byte below 90's column.
        expect(move("ArrowDown", 90)).toBe(90);
        expect(move("PageDown", 3)).toBe(67);
        expect(move("PageDown", 70)).toBe(70);
        expect(move("PageUp", 70)).toBe(6);
        expect(move("PageUp", 35)).toBe(3);
    });

    it("goes to the ends of the row, and of the file with Ctrl", () => {
        expect(move("Home", 21)).toBe(16);
        expect(move("End", 21)).toBe(31);
        expect(move("End", 97)).toBe(99);
        expect(move("Home", 21, true)).toBe(0);
        expect(move("End", 21, true)).toBe(99);
    });

    it("leaves every other key, and an empty file, alone", () => {
        expect(move("a", 5)).toBeNull();
        expect(move("Tab", 5)).toBeNull();
        expect(moveCursor("ArrowRight", false, 0, 0, 16, 4)).toBeNull();
    });
});

describe("the parts cache", () => {
    const file = Uint8Array.from({ length: CHUNK * 3 + 100 }, (_, i) => (i * 7) & 0xff);
    const reader = () => vi.fn(async (start: number, end: number) => file.slice(start, end + 1));

    it("reads a part once, whoever waits for it, and the last part to the file's end", async () => {
        const read = reader();
        const cache = new ChunkCache(read, file.length);
        expect(cache.byteAt(5)).toBeUndefined();
        await Promise.all([cache.ensure(0, 10), cache.ensure(5, 200)]);
        expect(read).toHaveBeenCalledTimes(1);
        expect(read).toHaveBeenCalledWith(0, CHUNK - 1);
        expect(cache.byteAt(5)).toBe(file[5]);
        await cache.ensure(CHUNK * 3, CHUNK * 3 + 5000);
        expect(read).toHaveBeenLastCalledWith(CHUNK * 3, file.length - 1);
        expect(cache.byteAt(file.length - 1)).toBe(file[file.length - 1]);
    });

    it("drops the part used longest ago", async () => {
        const cache = new ChunkCache(reader(), file.length, 2);
        await cache.ensure(0, 0);
        await cache.ensure(CHUNK, CHUNK);
        cache.touch(0, 0);
        await cache.ensure(CHUNK * 2, CHUNK * 2);
        expect(cache.has(0, 0)).toBe(true);
        expect(cache.has(CHUNK, CHUNK)).toBe(false);
        expect(cache.has(CHUNK * 2, CHUNK * 2)).toBe(true);
    });

    it("gives the bytes of a range that crosses parts, from the cache or from the file", async () => {
        const read = reader();
        const cache = new ChunkCache(read, file.length);
        const range: [number, number] = [CHUNK - 3, CHUNK + 2];
        expect(Array.from(await cache.bytes(...range))).toEqual(Array.from(file.slice(CHUNK - 3, CHUNK + 3)));
        expect(read).toHaveBeenCalledWith(...range);
        await cache.ensure(...range);
        read.mockClear();
        expect(Array.from(await cache.bytes(...range))).toEqual(Array.from(file.slice(CHUNK - 3, CHUNK + 3)));
        expect(read).not.toHaveBeenCalled();
        expect((await cache.bytes(file.length - 2, file.length + 50)).length).toBe(2);
        expect((await cache.bytes(file.length, file.length + 5)).length).toBe(0);
    });

    it("reads again after a failed read", async () => {
        const read = vi.fn<(s: number, e: number) => Promise<Uint8Array>>().mockRejectedValueOnce(new Error("gone")).mockResolvedValue(new Uint8Array(4));
        const cache = new ChunkCache(read, 4);
        await expect(cache.ensure(0, 3)).rejects.toThrow("gone");
        await cache.ensure(0, 3);
        expect(cache.has(0, 3)).toBe(true);
    });
});

describe("the data inspector", () => {
    const value = (bytes: number[], label: string, littleEndian = true) => inspect(Uint8Array.from(bytes), littleEndian).find((r) => r.label === label)?.value;

    it("reads the integers in either byte order", () => {
        const bytes = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x88];
        expect(value(bytes, "uint8")).toBe("1");
        expect(value(bytes, "uint16")).toBe("513");
        expect(value(bytes, "uint16", false)).toBe("258");
        expect(value(bytes, "uint32")).toBe("67305985");
        expect(value(bytes, "uint32", false)).toBe("16909060");
        expect(value(bytes, "uint64")).toBe("9801809732607083009");
        expect(value(bytes, "int64")).toBe("-8644934341102468607");
        expect(value(bytes, "uint64", false)).toBe("72623859790382984");
        expect(value([0xff], "int8")).toBe("-1");
        expect(value([0xff, 0xff], "int16")).toBe("-1");
        expect(value([0xfe, 0xff, 0xff, 0xff], "int32")).toBe("-2");
        expect(value([0xa5], "Binary")).toBe("10100101");
        expect(value([0xa5], "Octal")).toBe("245");
    });

    it("reads the floats, half precision included", () => {
        expect(float16(0x3c00)).toBe(1);
        expect(float16(0xc000)).toBe(-2);
        expect(float16(0x3555)).toBeCloseTo(0.33325, 5);
        expect(float16(0x0001)).toBeCloseTo(5.960464477539063e-8, 20);
        expect(float16(0x7c00)).toBe(Number.POSITIVE_INFINITY);
        expect(float16(0xfc00)).toBe(Number.NEGATIVE_INFINITY);
        expect(Number.isNaN(float16(0x7e00))).toBe(true);
        expect(value([0x00, 0x3c], "float16")).toBe("1");
        expect(value([0x3c, 0x00], "float16", false)).toBe("1");
        expect(value([0x00, 0x00, 0x80, 0x3f], "float32")).toBe("1");
        expect(value([0x3f, 0xf8, 0, 0, 0, 0, 0, 0], "float64", false)).toBe("1.5");
        expect(value([0x00, 0x7c], "float16")).toBe("+Infinity");
        expect(value([0x00, 0x7e], "float16")).toBe("NaN");
    });

    it("reads a ULEB128 number and a UTF-8 character", () => {
        expect(uleb128(Uint8Array.from([0xe5, 0x8e, 0x26]))).toBe("624485");
        expect(uleb128(Uint8Array.from([0x7f, 0xff]))).toBe("127");
        expect(uleb128(Uint8Array.from([0x80, 0x80]))).toBe("");
        expect(utf8Char(Uint8Array.from([0x41, 0x42]))).toBe("U+0041 A");
        expect(utf8Char(Uint8Array.from([0xc3, 0xa9]))).toBe("U+00E9 é");
        expect(utf8Char(Uint8Array.from([0xf0, 0x9f, 0x98, 0x80]))).toBe("U+1F600 😀");
        expect(utf8Char(Uint8Array.from([0x0a]))).toBe("U+000A");
        expect(utf8Char(Uint8Array.from([0xc3]))).toBe("");
        expect(utf8Char(Uint8Array.from([0xff, 0x41]))).toBe("");
        expect(utf8Char(Uint8Array.from([0xc3, 0x41]))).toBe("");
    });

    it("leaves a type empty when the file ends before it does", () => {
        expect(value([0x01, 0x02], "uint16")).toBe("513");
        expect(value([0x01, 0x02], "uint32")).toBe("");
        expect(value([0x01, 0x02], "float64")).toBe("");
        expect(value([], "uint8")).toBe("");
        expect(value([], "UTF-8")).toBe("");
    });
});

describe("what a copy writes", () => {
    const bytes = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x48, 0x69, 0x00]);

    it("writes hex pairs, a C array, base64 and the decoded text", () => {
        expect(asHex(bytes)).toBe("DE AD BE EF 48 69 00");
        expect(asCArray(bytes)).toBe("unsigned char data[7] = {\n    0xDE, 0xAD, 0xBE, 0xEF, 0x48, 0x69, 0x00\n};");
        expect(asBase64(bytes)).toBe("3q2+70hpAA==");
        expect(asText(bytes)).toBe("....Hi.");
        expect(copyText(bytes, "hex")).toBe(asHex(bytes));
        expect(copyText(bytes, "c")).toBe(asCArray(bytes));
        expect(copyText(bytes, "base64")).toBe(asBase64(bytes));
        expect(copyText(bytes, "text")).toBe(asText(bytes));
    });

    it("breaks a C array every twelve bytes, and takes a megabyte of base64", () => {
        const lines = asCArray(Uint8Array.from({ length: 25 }, (_, i) => i)).split("\n");
        expect(lines).toHaveLength(5);
        expect(lines[1].split(", ")).toHaveLength(12);
        expect(lines[3]).toBe("    0x18");
        const big = new Uint8Array(1_048_576).fill(0x41);
        expect(asBase64(big).length).toBe(1_398_104);
    });
});
