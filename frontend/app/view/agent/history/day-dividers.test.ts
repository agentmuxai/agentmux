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

    // Live-reproduced crash (SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md):
    // a real transcript's node timestamps aren't guaranteed strictly
    // monotonic (subagent transcript merges, tool-call-start vs. log-flush
    // stamps, retries) — a day visited, left, then revisited must not
    // produce the SAME `day-<key>` id twice. Two rows with an identical id
    // is a duplicate key in the virtualized list's `<Key by={r=>r.nodeId}>`,
    // which crashed `reconcileArrays` ("replaceChild: node not a child")
    // once real, large-volume history started loading.
    it("never emits the same day id twice, even if the day sequence goes back and forward again (non-monotonic timestamps)", () => {
        const out = injectDayDividers([
            md("a", day(2026, 8, 10)),
            md("b", day(2026, 8, 11)),
            md("c", day(2026, 8, 10)), // back to Aug 10 — must NOT re-divide
            md("d", day(2026, 8, 11)), // forward to Aug 11 — must NOT re-divide either
        ]);
        const dividerIds = out.filter((n) => n.type === "day_divider").map((n) => n.id);
        expect(dividerIds).toEqual(["day-2026-08-10", "day-2026-08-11"]);
        // No duplicate ids anywhere in the output, full stop — the
        // property <Key> actually depends on.
        const allIds = out.map((n) => n.id);
        expect(new Set(allIds).size).toBe(allIds.length);
        // The revisited-day node itself still renders, just without a
        // second boundary marker in front of it.
        expect(out.map((n) => n.type)).toEqual([
            "day_divider", "markdown", "day_divider", "markdown", "markdown", "markdown",
        ]);
    });
});
