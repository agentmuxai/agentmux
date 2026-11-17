// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { clearOpenPositions, OPEN_POSITION_TTL_MS, setOpenPosition, takeOpenPosition, whole } from "./open-position";
import { openPendingFiles, parsePendingEntry } from "./pending-open-files";

describe("open positions", () => {
    beforeEach(clearOpenPositions);

    it("gives each field once, to the pane and file it was noted for", () => {
        setOpenPosition("b1", "C:/a/x.pdf", { offset: 4096, page: 7 });
        expect(takeOpenPosition("b2", "C:/a/x.pdf", "page")).toBeUndefined();
        expect(takeOpenPosition("b1", "C:/a/y.pdf", "page")).toBeUndefined();
        expect(takeOpenPosition("b1", "C:/a/x.pdf", "page")).toBe(7);
        expect(takeOpenPosition("b1", "C:/a/x.pdf", "page")).toBeUndefined();
        expect(takeOpenPosition("b1", "C:/a/x.pdf", "offset")).toBe(4096);
        expect(takeOpenPosition("b1", "C:/a/x.pdf", "offset")).toBeUndefined();
    });

    it("matches a path however it is spelled", () => {
        setOpenPosition("b1", "C:\\a\\x.bin", { offset: 12 });
        expect(takeOpenPosition("b1", "C:/a/x.bin", "offset")).toBe(12);
    });

    it("keeps offset 0 as a request, and drops what isn't a whole number in range", () => {
        setOpenPosition("b1", "/f", { offset: 0 });
        expect(takeOpenPosition("b1", "/f", "offset")).toBe(0);
        setOpenPosition("b1", "/f", { offset: -1, page: 0 });
        expect(takeOpenPosition("b1", "/f", "offset")).toBeUndefined();
        expect(takeOpenPosition("b1", "/f", "page")).toBeUndefined();
        setOpenPosition("b1", "/f", { offset: 1.5, page: Number.NaN });
        expect(takeOpenPosition("b1", "/f", "offset")).toBeUndefined();
        expect(takeOpenPosition("b1", "/f", "page")).toBeUndefined();
    });

    it("lets a later open without a position clear an earlier one", () => {
        setOpenPosition("b1", "/f", { page: 3 });
        setOpenPosition("b1", "/f", {});
        expect(takeOpenPosition("b1", "/f", "page")).toBeUndefined();
    });

    it("lapses after a minute that no viewer took it", () => {
        const t0 = 1_000_000;
        setOpenPosition("b1", "/f", { page: 3 }, t0);
        expect(takeOpenPosition("b1", "/f", "page", t0 + OPEN_POSITION_TTL_MS + 1)).toBeUndefined();
        setOpenPosition("b1", "/f", { page: 3 }, t0);
        expect(takeOpenPosition("b1", "/f", "page", t0 + OPEN_POSITION_TTL_MS - 1)).toBe(3);
    });

    it("whole() takes whole numbers of at least min only", () => {
        expect([whole(3, 1), whole(0, 1), whole(0, 0), whole("3", 1), whole(2.5, 1), whole(null, 0)]).toEqual([3, undefined, 0, undefined, undefined, undefined]);
    });
});

describe("a pending entry's position", () => {
    it("carries a line, an offset and a page next to the viewer", () => {
        expect(parsePendingEntry({ path: "/a.bin", viewer: "hex", offset: 4096 })).toEqual({ path: "/a.bin", followFolder: false, viewer: "hex", offset: 4096 });
        expect(parsePendingEntry({ path: "/a.pdf", viewer: "pdf", page: 7 })).toEqual({ path: "/a.pdf", followFolder: false, viewer: "pdf", page: 7 });
        expect(parsePendingEntry({ path: "/a.rs", line: 42 })).toEqual({ path: "/a.rs", followFolder: false, line: 42 });
        expect(parsePendingEntry({ path: "/a.bin", offset: 0 })).toEqual({ path: "/a.bin", followFolder: false, offset: 0 });
    });

    it("ignores a position that is not a usable number", () => {
        expect(parsePendingEntry({ path: "/a", line: 0, offset: -5, page: "2" })).toEqual({ path: "/a", followFolder: false });
        expect(parsePendingEntry({ path: "/a", line: 1.5, offset: null, page: 0 })).toEqual({ path: "/a", followFolder: false });
    });

    it("reaches the open callback with the file", () => {
        const calls: unknown[][] = [];
        openPendingFiles([{ path: "/a.pdf", viewer: "pdf", page: 7 }], (path, opts) => calls.push([path, opts]), () => {});
        expect(calls).toEqual([["/a.pdf", { followFolder: false, viewer: "pdf", page: 7 }]]);
    });
});
