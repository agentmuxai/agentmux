// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's lines: pages read through a line index and a range
 * read, decoded, cut, and kept.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { describe, expect, it, vi } from "vitest";
import { decoderLabel, LINE_READ_BYTES, LINE_SHOWN_MAX, linesOfBytes, LineSource, PAGE_BYTES_MAX, PAGE_LINES } from "./line-source";

const enc = (s: string) => new TextEncoder().encode(s);

/** A file of `text` (UTF-8), served as srv's line index and a range read would. */
function fileOf(text: string, mtime = 1) {
    const bytes = enc(text);
    const starts = [0];
    for (let i = 0; i < bytes.length; i++) if (bytes[i] === 10 && i + 1 < bytes.length) starts.push(i + 1);
    const index = vi.fn(async (from: number, count: number) => {
        const at = (n: number) => (n >= starts.length ? bytes.length : starts[n]);
        return { total_lines: starts.length, size: bytes.length, mtime, start: at(from), end: at(from + count) };
    });
    const read = vi.fn(async (start: number, end: number) => bytes.slice(start, end + 1));
    return { index, read, lines: starts.length };
}

describe("linesOfBytes", () => {
    it("splits on line breaks, LF or CRLF, and a break at the end starts no line", () => {
        expect(linesOfBytes(enc("a\r\nb\n\nc\n"), "utf-8").map((l) => l.text)).toEqual(["a", "b", "", "c"]);
        expect(linesOfBytes(enc("a\nb"), "utf-8").map((l) => l.text)).toEqual(["a", "b"]);
    });

    it("decodes UTF-16 and a legacy code page", () => {
        const le = Uint8Array.from("hé\nx".split("").flatMap((c) => [c.charCodeAt(0), 0]));
        expect(linesOfBytes(le, "utf-16le").map((l) => l.text)).toEqual(["hé", "x"]);
        expect(linesOfBytes(Uint8Array.from([0x63, 0x61, 0x66, 0xe9]), "legacy")[0].text).toBe("café");
        expect(decoderLabel(undefined)).toBe("utf-8");
    });

    it("cuts a line over the limit and says how much is hidden", () => {
        const [line] = linesOfBytes(enc("x".repeat(LINE_SHOWN_MAX + 1234)), "utf-8");
        expect(line.text).toHaveLength(LINE_SHOWN_MAX);
        expect(line.hidden).toBe("+1,234 characters");
    });
});

describe("LineSource", () => {
    const text = Array.from({ length: 450 }, (_, i) => `line ${i + 1}`).join("\n") + "\n";

    it("reads the page that holds the lines asked for, once, and learns the file's lines", async () => {
        const f = fileOf(text);
        const s = new LineSource(f.index, f.read, "utf-8");
        expect(s.total).toBe(-1);
        await Promise.all([s.ensure(250, 260), s.ensure(255, 256)]);
        expect(s.total).toBe(450);
        expect(s.lineAt(250)?.text).toBe("line 251");
        expect(s.lineAt(10)).toBeUndefined();
        expect(f.index).toHaveBeenCalledTimes(1);
        expect(f.index).toHaveBeenCalledWith(PAGE_LINES, PAGE_LINES);
        expect(f.read).toHaveBeenCalledTimes(1);
        expect(s.has(250, 399)).toBe(true);
        expect(s.has(250, 400)).toBe(false);
        // The page after it is not here.
        expect(s.has(400, 449)).toBe(false);
    });

    it("reads no page past the last line", async () => {
        const f = fileOf(text);
        const s = new LineSource(f.index, f.read, "utf-8");
        await s.ensure(0, 0);
        await s.ensure(440, 100_000);
        expect(s.lineAt(449)?.text).toBe("line 450");
        expect(s.lineAt(450)).toBeUndefined();
        expect(f.index).toHaveBeenCalledTimes(2);
    });

    it("lets the least recently used page go", async () => {
        const f = fileOf(text);
        const s = new LineSource(f.index, f.read, "utf-8", 2);
        await s.ensure(0, 0);
        await s.ensure(200, 200);
        s.touch(0, 0);
        await s.ensure(400, 400);
        expect(s.has(0, 0)).toBe(true);
        expect(s.has(200, 200)).toBe(false);
        expect(s.has(400, 400)).toBe(true);
    });

    it("drops what it read of a file that changed", async () => {
        const f = fileOf(text, 1);
        const s = new LineSource(f.index, f.read, "utf-8");
        await s.ensure(0, 0);
        f.index.mockImplementation(async () => ({ total_lines: 450, size: 5, mtime: 2, start: 0, end: 0 }));
        await s.ensure(200, 200);
        expect(s.has(0, 0)).toBe(false);
    });

    it("reads a page with a huge line one line at a time, each line's start only", async () => {
        const big = "y".repeat(PAGE_BYTES_MAX + 10);
        const f = fileOf(`first\n${big}\nlast\n`);
        const s = new LineSource(f.index, f.read, "utf-8");
        await s.ensure(0, 2);
        expect(s.lineAt(0)?.text).toBe("first");
        expect(s.lineAt(1)?.text).toHaveLength(LINE_SHOWN_MAX);
        expect(s.lineAt(1)?.hidden).toMatch(/^\+4(\.\d+)? MB$/);
        expect(s.lineAt(2)?.text).toBe("last");
        // Nothing read was bigger than a line's start.
        for (const [a, b] of f.read.mock.calls) expect(b - a + 1).toBeLessThanOrEqual(LINE_READ_BYTES);
    });

    it("reports a read that fails, and tries again when asked again", async () => {
        const f = fileOf(text);
        f.read.mockRejectedValueOnce(new Error("gone"));
        const s = new LineSource(f.index, f.read, "utf-8");
        await expect(s.ensure(0, 0)).rejects.toThrow("gone");
        await s.ensure(0, 0);
        expect(s.lineAt(0)?.text).toBe("line 1");
    });
});
