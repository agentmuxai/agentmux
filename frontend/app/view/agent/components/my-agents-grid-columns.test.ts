// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { columnsForWidth, gridMetrics } from "./my-agents-grid-columns";

const style = (columnGap: string, tile: string) => ({ columnGap, getPropertyValue: () => tile });

describe("columnsForWidth", () => {
    const metrics = { tile: 240, gap: 8 };

    it("fits as many minimum-width tiles and gaps as the width holds", () => {
        expect(columnsForWidth(240, metrics)).toBe(1);
        expect(columnsForWidth(487, metrics)).toBe(1);
        expect(columnsForWidth(488, metrics)).toBe(2);
        expect(columnsForWidth(1000, metrics)).toBe(4);
    });

    it("is one column when the grid is narrower than a tile (min(tile, 100%))", () => {
        expect(columnsForWidth(120, metrics)).toBe(1);
        expect(columnsForWidth(0, metrics)).toBe(1);
    });
});

describe("gridMetrics", () => {
    it("reads the tile minimum and gap when both are plain pixel lengths", () => {
        expect(gridMetrics(style("8px", "240px"))).toEqual({ tile: 240, gap: 8 });
        expect(gridMetrics(style("8.5px", " 480px "))).toEqual({ tile: 480, gap: 8.5 });
    });

    it("uses the stylesheet's 480px when the tile minimum is not set", () => {
        expect(gridMetrics(style("8px", ""))).toEqual({ tile: 480, gap: 8 });
    });

    it("gives up on anything the browser has to resolve", () => {
        expect(gridMetrics(style("normal", "240px"))).toBeNull();
        expect(gridMetrics(style("8px", "calc(20rem)"))).toBeNull();
        expect(gridMetrics({ columnGap: "8px" } as never)).toEqual({ tile: 480, gap: 8 });
    });
});
