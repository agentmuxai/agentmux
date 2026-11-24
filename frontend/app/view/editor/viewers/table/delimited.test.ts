// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The delimited-text parser behind the Table viewer. */

import { describe, expect, it } from "vitest";
import { columnCount, columnWidths, parseDelimited, sniffDelimiter } from "./delimited";

describe("parseDelimited", () => {
    it("splits cells and rows, on LF, CRLF or CR, and a break at the end starts no row", () => {
        expect(parseDelimited("a,b\n1,2\n", ",")).toEqual([["a", "b"], ["1", "2"]]);
        expect(parseDelimited("a,b\r\n1,2\r\n", ",")).toEqual([["a", "b"], ["1", "2"]]);
        expect(parseDelimited("a,b\r1,2", ",")).toEqual([["a", "b"], ["1", "2"]]);
        expect(parseDelimited("", ",")).toEqual([]);
    });

    it("keeps empty cells, a last empty cell, and a blank line as a row of one empty cell", () => {
        expect(parseDelimited("a,,c\n,\n\nx", ",")).toEqual([["a", "", "c"], ["", ""], [""], ["x"]]);
        expect(parseDelimited("a,b,", ",")).toEqual([["a", "b", ""]]);
    });

    it("reads quoted cells: the delimiter, doubled quotes and line breaks stay in the cell", () => {
        expect(parseDelimited('"a,b","say ""hi""","line1\nline2"\n', ",")).toEqual([["a,b", 'say "hi"', "line1\nline2"]]);
        expect(parseDelimited('a,"",b', ",")).toEqual([["a", "", "b"]]);
        // A quote inside a plain cell is just a character.
        expect(parseDelimited('5" pipe,x', ",")).toEqual([['5" pipe', "x"]]);
    });

    it("takes an unterminated quote to the end of the text, and drops a byte order mark", () => {
        expect(parseDelimited('a,"b\nc', ",")).toEqual([["a", "b\nc"]]);
        expect(parseDelimited("\uFEFFa,b\n", ",")).toEqual([["a", "b"]]);
    });

    it("splits on the delimiter it is given", () => {
        expect(parseDelimited("a\tb\n1,5\t2\n", "\t")).toEqual([["a", "b"], ["1,5", "2"]]);
        expect(parseDelimited("a;b|c\n", ";")).toEqual([["a", "b|c"]]);
    });
});

describe("sniffDelimiter", () => {
    it("picks the delimiter that splits every first line into the same cells", () => {
        expect(sniffDelimiter("a,b,c\n1,2,3\n")).toBe(",");
        expect(sniffDelimiter("a\tb\tc\n1\t2\t3\n")).toBe("\t");
        expect(sniffDelimiter("a;b;c\n1;2;3\n")).toBe(";");
        expect(sniffDelimiter("a|b\n1|2\n")).toBe("|");
    });

    it("goes by the line with the fewest, so a comma in a note doesn't win over the real delimiter", () => {
        expect(sniffDelimiter("name;note\nx;hello, world\ny;ok\n")).toBe(";");
    });

    it("falls back to a comma, and honours a hint that fits", () => {
        expect(sniffDelimiter("just words\nmore words\n")).toBe(",");
        expect(sniffDelimiter("a\tb,c\n1\t2,3\n", "\t")).toBe("\t");
        expect(sniffDelimiter("a,b\n1,2\n", "\t")).toBe(",");
    });
});

describe("columns", () => {
    it("counts the widest row and sizes columns from the first rows, within limits", () => {
        const rows = [["id", "name"], ["1", "a very long cell indeed, past the limit of the column's width"], ["2"]];
        expect(columnCount(rows)).toBe(2);
        expect(columnWidths(rows, 2)).toEqual([4, 40]);
        expect(columnWidths(rows, 3, 2, 10)).toEqual([2, 10, 2]);
    });
});
