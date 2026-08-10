// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { injectDayDividers } from "./day-dividers";
import type { DocumentNode } from "../types";

const md = (id: string, timestamp?: number): DocumentNode => ({
    type: "markdown",
    id,
    content: id,
    ...(timestamp != null ? { timestamp } : {}),
});

// Local-noon timestamps avoid any timezone edge in the expectations.
const day = (y: number, m: number, d: number): number => new Date(y, m - 1, d, 12).getTime();

describe("injectDayDividers (§4.4)", () => {
    it("inserts a divider at each local-day change", () => {
        const out = injectDayDividers([
            md("a", day(2026, 8, 9)),
            md("b", day(2026, 8, 9)),
            md("c", day(2026, 8, 10)),
        ]);
        expect(out.map((n) => n.type)).toEqual([
            "day_divider", "markdown", "markdown", "day_divider", "markdown",
        ]);
        expect(out[0].id).toBe("day-2026-08-09");
        expect(out[3].id).toBe("day-2026-08-10");
    });

    it("carries the last known day forward across untimestamped nodes", () => {
        const out = injectDayDividers([
            md("a", day(2026, 8, 9)),
            md("b"), // unknown — inherits Aug 9, no divider
            md("c", day(2026, 8, 9)),
        ]);
        expect(out.filter((n) => n.type === "day_divider")).toHaveLength(1);
    });

    it("leading untimestamped nodes produce no divider", () => {
        const out = injectDayDividers([md("a"), md("b", day(2026, 8, 10))]);
        expect(out.map((n) => n.type)).toEqual(["markdown", "day_divider", "markdown"]);
    });

    it("ids are stable across recomputation with prepended pages", () => {
        const newer = [md("c", day(2026, 8, 10))];
        const first = injectDayDividers(newer);
        const merged = injectDayDividers([md("a", day(2026, 8, 9)), ...newer]);
        const idsIn = (nodes: DocumentNode[]) =>
            nodes.filter((n) => n.type === "day_divider").map((n) => n.id);
        expect(idsIn(first)).toEqual(["day-2026-08-10"]);
        expect(idsIn(merged)).toEqual(["day-2026-08-09", "day-2026-08-10"]);
    });

    it("empty input → empty output", () => {
        expect(injectDayDividers([])).toEqual([]);
    });
});
