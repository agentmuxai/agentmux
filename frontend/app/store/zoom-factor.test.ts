// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { clampZoom, readZoom } from "./zoom-factor";

describe("readZoom", () => {
    it("reads a pane's term:zoom, clamped to 0.5..2.0", () => {
        expect(readZoom({ "term:zoom": 1.25 })).toBe(1.25);
        expect(readZoom({ "term:zoom": 0.1 })).toBe(0.5);
        expect(readZoom({ "term:zoom": 9 })).toBe(2.0);
    });

    it("is 1.0 when unset, not a number, or NaN", () => {
        for (const meta of [undefined, null, {}, { "term:zoom": null }, { "term:zoom": "1.5" }, { "term:zoom": NaN }]) {
            expect(readZoom(meta)).toBe(1.0);
        }
    });
});

describe("clampZoom", () => {
    it("clamps without rounding", () => {
        expect(clampZoom(0.333)).toBe(0.5);
        expect(clampZoom(1.234)).toBe(1.234);
        expect(clampZoom(3)).toBe(2.0);
    });
});
