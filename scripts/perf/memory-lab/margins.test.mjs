// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { measuresComparison } from "./lib/measures.mjs";
import { compareRuns, DEFAULT_MARGINS, runsTable } from "./lib/summary.mjs";

describe("the margins the nightly performance check flags at", () => {
    const run = (label, over = {}) => ({
        label,
        steady: { app: { privateMB: 900 }, groups: { renderer: { privateMB: 600 } } },
        cpu: { seconds: { app: 400, srv: 100, renderer: 250, gpu: 30 } },
        wall: { medianMs: 15000 },
        io: { writtenMB: { srv: 14000 } },
        ...over,
    });
    const times = (n, f) => Array.from({ length: n }, (_, i) => f(i));

    it("are the decided ones", () => {
        expect(DEFAULT_MARGINS).toEqual({ cpuPct: { app: 10, srv: 10, renderer: 10, gpu: 20, default: 20 }, wallPct: 20, writtenPct: 3, memoryPct: null });
    });

    it("flag srv bytes written at +3% (the best signal: it varies 0.2-0.4% between runs)", () => {
        const base = times(4, (i) => run(`b${i}`, { io: { writtenMB: { srv: 14000 + i } } }));
        const head = times(4, (i) => run(`h${i}`, { io: { writtenMB: { srv: 14500 + i } } })); // +3.6%
        expect(compareRuns(base, head).written.flagged).toBe(true);
        const near = times(4, (i) => run(`n${i}`, { io: { writtenMB: { srv: 14300 + i } } })); // +2.1%
        expect(compareRuns(base, near).written.flagged).toBe(false);
    });

    it("flag GPU CPU only past +20%, and renderer CPU past +10%", () => {
        const base = times(4, (i) => run(`b${i}`, { cpu: { seconds: { app: 400, srv: 100, renderer: 250 + i, gpu: 30 + i * 0.1 } } }));
        const head = times(4, (i) => run(`h${i}`, { cpu: { seconds: { app: 400, srv: 100, renderer: 290 + i, gpu: 36 + i * 0.1 } } })); // renderer +16%, gpu +19.5%
        const cmp = compareRuns(base, head);
        expect(cmp.rows.find((r) => r.group === "renderer").cpu.flagged).toBe(true);
        expect(cmp.rows.find((r) => r.group === "gpu").cpu.flagged).toBe(false);
        const worse = times(4, (i) => run(`w${i}`, { cpu: { seconds: { app: 400, srv: 100, renderer: 250, gpu: 38 + i * 0.1 } } })); // gpu +27%
        expect(compareRuns(base, worse).rows.find((r) => r.group === "gpu").cpu.flagged).toBe(true);
    });

    it("keep memory report-only until the margins name one, then flag it past that, with the ranges apart", () => {
        const base = times(4, (i) => run(`b${i}`, { steady: { app: { privateMB: 900 + i }, groups: { renderer: { privateMB: 600 } } } }));
        const head = times(4, (i) => run(`h${i}`, { steady: { app: { privateMB: 1100 + i }, groups: { renderer: { privateMB: 600 } } } })); // +22%
        const report = compareRuns(base, head);
        expect(report.rows.find((r) => r.group === "app").memory.flagged).toBe(false);
        const flagged = compareRuns(base, head, { ...DEFAULT_MARGINS, memoryPct: 10 });
        expect(flagged.rows.find((r) => r.group === "app").memory.flagged).toBe(true);
        expect(runsTable(flagged, "base", "head")).toMatch(/memory over \+10%/);
        // overlapping ranges are noise, even past the margin
        const noisy = [run("n1", { steady: { app: { privateMB: 800 }, groups: {} } }), run("n2", { steady: { app: { privateMB: 1150 }, groups: {} } })];
        const noisy2 = [run("m1", { steady: { app: { privateMB: 950 }, groups: {} } }), run("m2", { steady: { app: { privateMB: 1200 }, groups: {} } })];
        expect(compareRuns(noisy, noisy2, { ...DEFAULT_MARGINS, memoryPct: 5 }).rows.find((r) => r.group === "app").memory.flagged).toBe(false);
    });

    it("come out as the history line's measures, under its names, with the flagged ones listed", () => {
        const base = times(4, (i) => run(`b${i}`, { io: { writtenMB: { srv: 14000 + i } }, slowEvals: { total: 1 }, storeLock: { longestHeldMs: 0 } }));
        const head = times(4, (i) => run(`h${i}`, { io: { writtenMB: { srv: 14600 + i } }, slowEvals: { total: 3 }, storeLock: { longestHeldMs: 900 } }));
        const { measures, flags } = measuresComparison(compareRuns(base, head));
        expect(Object.keys(measures)).toEqual([
            "app_mb", "renderer_mb", "srv_mb", "app_cpu_s", "srv_cpu_s", "renderer_cpu_s", "gpu_cpu_s", "round_ms", "srv_written_mb", "store_lock_max_ms", "slow_page_commands", "open_first_screen_ms", "open_heavy_first_screen_ms", "open_longest_task_ms",
        ]);
        expect(measures.srv_written_mb).toMatchObject({ flag: true });
        expect(measures.srv_written_mb.a).toBeCloseTo(14001.5);
        expect(measures.srv_written_mb.change_pct).toBeCloseTo(4.3, 1);
        expect(measures.app_cpu_s).toEqual({ a: 400, b: 400, change_pct: 0, flag: false });
        expect(flags).toEqual(["srv_written_mb"]);
        expect(measures.slow_page_commands).toMatchObject({ a: 1, b: 3, flag: false }); // reported, not flagged
        // a measure the runs have no figure for is null on both sides, never zero
        const bare = measuresComparison(compareRuns([{ label: "x", steady: { app: { privateMB: 1 }, groups: {} } }], [{ label: "y", steady: { app: { privateMB: 1 }, groups: {} } }]));
        expect(bare.measures.srv_written_mb).toEqual({ a: null, b: null, change_pct: null, flag: false });
    });

    it("fill a CPU margin file from the defaults, so a group it leaves out still flags", async () => {
        const { spawnSync } = await import("node:child_process");
        const { mkdtempSync, writeFileSync, rmSync } = await import("node:fs");
        const { tmpdir } = await import("node:os");
        const { join } = await import("node:path");
        const dir = mkdtempSync(join(tmpdir(), "lab-margins-"));
        try {
            const mk = (name, app) => {
                const path = join(dir, name);
                writeFileSync(path, JSON.stringify({ ...run(name), layout: { panes: 12, tabs: [12] }, rounds: 10, seed: 1, gapSeconds: 15, platform: "win32", settleSeconds: 20, steady: { samples: 3, app: { privateMB: 900 }, groups: { renderer: { privateMB: 600 } } }, cpu: { seconds: { app, srv: 100, renderer: 250, gpu: 30 } } }));
                return path;
            };
            const margins = join(dir, "m.json");
            writeFileSync(margins, JSON.stringify({ margins: { cpuPct: { gpu: 50 } } })); // names only gpu
            const out = join(dir, "out.json");
            const base = `${mk("a1", 400)},${mk("a2", 401)}`;
            const head = `${mk("b1", 450)},${mk("b2", 451)}`; // app CPU +12%
            spawnSync(process.execPath, [join(process.cwd(), "scripts/perf/memory-lab/compare.mjs"), "--base", base, "--head", head, "--margins", margins, "--json", out], { encoding: "utf8" });
            const { readFileSync } = await import("node:fs");
            expect(JSON.parse(readFileSync(out, "utf8")).flags).toContain("app_cpu_s");
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    });
});
