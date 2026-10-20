// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { appendHistory, chartOf, driftOf, driftOnset, historyLine, isStandardWorkload, median, parseHistory, trendMarkdown } from "./lib/perfhistory.mjs";

const cell = (a, b, changePct, flag = false) => ({ a, b, change_pct: changePct, flag });
const plan = {
    a: { id: 111, sha: "71a3e2300aaaa", createdAt: "2026-10-18T11:00:00Z" },
    b: { id: 112, sha: "5ba04f91fbbbb", createdAt: "2026-10-19T11:00:00Z" },
};
const workload = { version: 2, panes: 12, rounds: 10 };
const final = {
    base: ["a1", "a2", "a3", "a4", "a1"],
    head: ["b1", "b2", "b3", "b4", "b1"],
    flags: ["srv_cpu_s"],
    measures: { app_mb: cell(640.123, 652.987, 2.014), srv_cpu_s: cell(38, 44.1, 16.123, true), open_first_screen_ms: null },
};
const initial = { base: ["a1", "a2", "a3", "a4"], head: ["b1", "b2", "b3", "b4"] };
const base = { date: "2026-10-20", run: { id: 7, url: "https://example/run/7" }, plan, versions: { a: "0.59.34", b: "0.59.34" }, workload, initial, final, issue: 5200 };

describe("a night's history line", () => {
    it("has the spec's fields: builds by commit, workload, rounded medians, flags, whether the pair confirmed, the issue", () => {
        const line = historyLine(base);
        expect(line).toEqual({
            date: "2026-10-20",
            run_id: 7,
            run_url: "https://example/run/7",
            a: { commit: "71a3e2300", version: "0.59.34", artifacts_run_id: 111 },
            b: { commit: "5ba04f91f", version: "0.59.34", artifacts_run_id: 112 },
            workload: { version: 2, panes: 12, rounds: 10, runs_per_side: 5 },
            measures: {
                app_mb: { a: 640.12, b: 652.99, change_pct: 2, flag: false },
                srv_cpu_s: { a: 38, b: 44.1, change_pct: 16.1, flag: true },
            },
            flags: ["srv_cpu_s"],
            confirmed: true,
            issue: 5200,
        });
    });

    it("leaves out measures the lab did not produce", () => {
        expect(Object.keys(historyLine(base).measures)).not.toContain("open_first_screen_ms");
    });

    it("has a null `a` and no measures on a night with no earlier build", () => {
        const line = historyLine({ ...base, plan: { ...plan, a: null }, final: null, initial: null, issue: null });
        expect(line.a).toBeNull();
        expect(line.b.commit).toBe("5ba04f91f");
        expect(line.measures).toEqual({});
        expect(line.flags).toEqual([]);
        expect(line.confirmed).toBe(false);
        expect(line.issue).toBeNull();
    });

    it("is not confirmed when nothing flagged, or when the pair added no runs", () => {
        expect(historyLine({ ...base, final: { ...final, flags: [], measures: { app_mb: cell(1, 1, 0) } } }).confirmed).toBe(false);
        expect(historyLine({ ...base, initial: { base: final.base, head: final.head } }).confirmed).toBe(false);
        expect(historyLine({ ...base, final: { ...final, base: initial.base } }).confirmed).toBe(false); // B1 finished, A1 did not
    });
});

describe("the workload", () => {
    it("is standard at 12 agents and 10 rounds, whatever else", () => {
        expect(isStandardWorkload({ panes: 12, rounds: 10 })).toBe(true);
        expect(isStandardWorkload({ panes: 4, rounds: 10 })).toBe(false);
        expect(isStandardWorkload({ panes: 12, rounds: 3 })).toBe(false);
        expect(isStandardWorkload(null)).toBe(false);
    });
});

describe("the history file", () => {
    it("appends a line, keeps the others, and replaces the line of a rerun of the same run", () => {
        const one = appendHistory("", historyLine(base));
        const two = appendHistory(one, historyLine({ ...base, date: "2026-10-21", run: { id: 8, url: "u" } }));
        expect(two.trim().split("\n")).toHaveLength(2);
        expect(two.endsWith("\n")).toBe(true);
        const rerun = appendHistory(two, historyLine({ ...base, run: { id: 8, url: "u" }, date: "2026-10-21", issue: null }));
        const lines = parseHistory(rerun);
        expect(lines.map((l) => l.run_id)).toEqual([7, 8]);
    });

    it("skips a damaged line instead of failing", () => {
        const text = `${JSON.stringify(historyLine(base))}\n{"date": "2026-10\n\n`;
        expect(parseHistory(text)).toHaveLength(1);
    });
});

/** `n` consecutive nights ending 2026-10-31 or so, each with the given B figure for app_mb (the others follow it). */
const nights = (values, name = "app_mb") =>
    values.map((v, i) => ({
        date: `2026-09-${String(i + 1).padStart(2, "0")}`,
        run_id: i + 1,
        a: { commit: "a" },
        workload: { version: 2, panes: 12, rounds: 10, runs_per_side: 4 },
        measures: { [name]: { a: v, b: v, change_pct: 0, flag: false } },
    }));

describe("the drift check", () => {
    const flat = Array(30).fill(640);

    it("flags a measure whose last 7 nights' median is more than 15% above the 30 nights before", () => {
        const drift = driftOf(nights([...flat, ...Array(7).fill(760)]));
        expect(drift).toEqual([{ name: "app_mb", recent: 760, before: 640, changePct: 18.8 }]);
    });

    it("does not flag 15% or less, or a fall", () => {
        expect(driftOf(nights([...flat, ...Array(7).fill(736)]))).toEqual([]); // exactly +15%
        expect(driftOf(nights([...flat, ...Array(7).fill(500)]))).toEqual([]);
    });

    it("uses medians, so one bad night in the last seven does not make a drift", () => {
        expect(driftOf(nights([...flat, 640, 640, 640, 640, 640, 640, 2000]))).toEqual([]);
    });

    it("says nothing with too few nights to know", () => {
        expect(driftOf(nights([...Array(5).fill(640), ...Array(7).fill(760)]))).toEqual([]);
        expect(driftOf(nights([...Array(30).fill(640), ...Array(3).fill(760)]))).toEqual([]);
    });

    it("compares only nights of the latest night's workload, and one night per date", () => {
        const old = nights(flat).map((l) => ({ ...l, workload: { ...l.workload, version: 1 } }));
        const recent = nights(Array(7).fill(760)).map((l, i) => ({ ...l, date: `2026-10-${String(i + 1).padStart(2, "0")}`, run_id: 100 + i }));
        expect(driftOf([...old, ...recent])).toEqual([]); // nothing to compare the recent nights with
        const twice = nights([...flat, ...Array(7).fill(760)]);
        expect(driftOf([...twice, { ...twice[twice.length - 1], run_id: 999 }])).toHaveLength(1); // a second run on a date counts once
    });

    it("skips nights with no earlier build", () => {
        // the last seven nights had nothing to compare (no `a`), so there are no recent figures to take a median of
        const history = nights([...flat, ...Array(7).fill(760)]).map((l, i) => (i >= 30 ? { ...l, a: null, measures: {} } : l));
        expect(driftOf(history)).toEqual([]);
    });

    it("reports a drift on the night it starts: the fourth high night of seven tips the median", () => {
        const all = nights([...flat, 760, 760, 760, 760]);
        const prior = all.slice(0, -1);
        expect(driftOf(prior)).toEqual([]);
        const r = driftOnset(prior, all[all.length - 1]);
        expect(r.current.map((d) => d.name)).toEqual(["app_mb"]);
        expect(r.onset.map((d) => d.name)).toEqual(["app_mb"]);
    });

    it("does not report it again on the nights it lasts", () => {
        const all = nights([...flat, 760, 760, 760, 760, 760]);
        const r = driftOnset(all.slice(0, -1), all[all.length - 1]);
        expect(r.current.map((d) => d.name)).toEqual(["app_mb"]);
        expect(r.onset).toEqual([]);
    });
});

describe("the charts", () => {
    it("draws a Mermaid xychart of B's figure each night, with dates as MM-DD labels", () => {
        const md = chartOf(nights([600, 620, 640]), "app_mb");
        expect(md).toContain("```mermaid\nxychart-beta");
        expect(md).toContain('x-axis ["09-01", "09-02", "09-03"]');
        expect(md).toContain("line [600, 620, 640]");
        expect(md).toMatch(/y-axis "MB" \d+ --> \d+/);
        expect(md.trim().endsWith("```")).toBe(true);
    });

    it("keeps the last 30 nights", () => {
        const md = chartOf(nights(Array.from({ length: 40 }, (_, i) => 600 + i)), "app_mb");
        expect(md.match(/"09-\d\d"/g)).toHaveLength(30);
        expect(md).toContain("last 30 nights");
    });

    it("draws nothing for fewer than two nights, and the trend section says so", () => {
        expect(chartOf(nights([640]), "app_mb")).toBe("");
        expect(trendMarkdown(nights([640]))).toContain("Not enough nights");
    });

    it("names a drift under the charts", () => {
        const md = trendMarkdown(nights([600, 620]), [{ name: "app_mb", recent: 760, before: 640, changePct: 18.8 }]);
        expect(md).toContain("Slow drift");
        expect(md).toContain("app_mb (+18.8%)");
    });
});

describe("median", () => {
    it("is the middle value, the mean of the two middle ones, or null", () => {
        expect([median([3, 1, 2]), median([4, 1, 3, 2]), median([]), median([null, 5])]).toEqual([2, 2.5, null, 5]);
    });
});
