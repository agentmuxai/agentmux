// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseSeedZoom } from "./agent-zoom-seed";

describe("parseSeedZoom", () => {
    it("returns null for missing/empty/whitespace input", () => {
        expect(parseSeedZoom(undefined)).toBeNull();
        expect(parseSeedZoom(null)).toBeNull();
        expect(parseSeedZoom("")).toBeNull();
        expect(parseSeedZoom("   ")).toBeNull();
    });

    it("returns null for the default zoom (1)", () => {
        expect(parseSeedZoom("1")).toBeNull();
        expect(parseSeedZoom("1.0")).toBeNull();
        expect(parseSeedZoom(" 1 ")).toBeNull();
    });

    it("returns null for out-of-range values", () => {
        expect(parseSeedZoom("0.49")).toBeNull();
        expect(parseSeedZoom("2.01")).toBeNull();
        expect(parseSeedZoom("0")).toBeNull();
        expect(parseSeedZoom("-1")).toBeNull();
        expect(parseSeedZoom("100")).toBeNull();
    });

    it("returns null for unparseable garbage", () => {
        expect(parseSeedZoom("not-a-number")).toBeNull();
        expect(parseSeedZoom("NaN")).toBeNull();
        expect(parseSeedZoom("1.5x")).toBeNull();
    });

    it("returns the parsed value for valid in-range, non-default zooms", () => {
        expect(parseSeedZoom("1.5")).toBe(1.5);
        expect(parseSeedZoom("0.5")).toBe(0.5);
        expect(parseSeedZoom("2")).toBe(2);
        expect(parseSeedZoom(" 1.25 ")).toBe(1.25);
    });
});
