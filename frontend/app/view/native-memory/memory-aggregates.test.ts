// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    fileDistribution,
    fileDistributionLabel,
    fleetSummary,
    fleetSummaryLabel,
    median,
    percentile,
    topContributors,
    topContributorsLabel,
} from "./memory-aggregates";

describe("memory-aggregates", () => {
    it("takes the median and p90 of small sets", () => {
        expect([median([]), percentile([], 90)]).toEqual([0, 0]);
        expect([median([7]), percentile([7], 90)]).toEqual([7, 7]);
        expect([median([10, 2]), percentile([10, 2], 90)]).toEqual([6, 10]);
        expect([median([3, 1, 2]), percentile([3, 1, 2], 90)]).toEqual([2, 3]);
        const ten = [1, 2, 3, 4, 5, 6, 7, 8, 9, 100];
        expect([median(ten), percentile(ten, 90)]).toEqual([5.5, 9]);
    });

    it("gives the largest entries' share only when there are more than that many", () => {
        expect(topContributors([5, 4, 3])).toBeNull();
        expect(topContributors([0, 0, 0, 0])).toBeNull();
        const t = topContributors([100, 50, 600, 200, 50]);
        expect(t).toEqual({ count: 3, tokens: 900, total: 1000 });
        expect(topContributorsLabel(t!, "entries", "Global Memory")).toBe("The 3 largest entries are 90% of Global Memory");
    });

    it("counts the files over p90", () => {
        expect(fileDistribution([])).toBeNull();
        expect(fileDistribution([40])).toEqual({ files: 1, median: 40, p90: 40, overP90: 0 });
        const d = fileDistribution([1, 2, 3, 4, 5, 6, 7, 8, 9, 100, 4000]);
        expect(d).toMatchObject({ files: 11, median: 6, p90: 100, overP90: 1 });
        expect(fileDistributionLabel(d!)).toBe("Per file: median ~6 tok, p90 ~100 tok · 1 file over p90");
        expect(fileDistributionLabel(fileDistribution([40])!)).toBe("Per file: median ~40 tok, p90 ~40 tok");
    });

    it("sums the fleet, takes the median over agents with memory and lists those over twice it", () => {
        expect(fleetSummary([{ id: "a", name: "A", tokens: 0 }])).toBeNull();
        const f = fleetSummary([
            { id: "a", name: "A", tokens: 1000 },
            { id: "b", name: "B", tokens: 0 },
            { id: "c", name: "C", tokens: 1200 },
            { id: "d", name: "D", tokens: 9000 },
            { id: "e", name: "E", tokens: 2600 },
        ]);
        expect(f).toMatchObject({ agents: 5, withMemory: 4, total: 13_800, medianPerAgent: 1900 });
        expect(f!.outliers.map((a) => a.id)).toEqual(["d"]);
        expect(fleetSummaryLabel(f!)).toBe("~14k tok across 4 of 5 agents · median ~1.9k tok per agent");
    });
});
