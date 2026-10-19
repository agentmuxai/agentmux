// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the memory lab's grouping, steady values, comparison and
// conversation (scripts/perf/memory-lab). Starting and driving an instance
// needs a build and isn't tested here.

import { describe, expect, it } from "vitest";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";

import { mismatch } from "./compare.mjs";
import { evaluate, report } from "./gate.mjs";
import { driftWarning, portableCommit, srvTarget, withReplacedSrv } from "./lib/srv.mjs";
import { mergeLongTaskRecords, slowSendsAgainstLongTasks, summariseLongTasks } from "./lib/longtasks.mjs";
import { parseDiskLoad, sleepNeededMs, startDiskLoad } from "./lib/diskload.mjs";
import { volumeOf } from "./lib/volume.mjs";
import { buildInstanceEnv, parseEnvPairs, RESERVED } from "./lib/instanceenv.mjs";
import { spawnSync } from "node:child_process";
import { measuresOf, spreadMarkdown, spreadOf, spreadTable } from "./lib/measures.mjs";
import { segmentsOf, slowSendBreakdown, timedCall } from "./lib/sendsegments.mjs";
import { arrivalTotals, arrivalsBetween, arrivalsDuringSlowSends } from "./lib/wsarrivals.mjs";
import { mergeStoreLock, parseStoreLock } from "./lib/storelock.mjs";
import { CpuLedger, cpuBetween, writtenBetween } from "./lib/cpu.mjs";
import { generateConversation } from "./lib/conversation.mjs";
import { commandName, portableExe } from "./lib/driver.mjs";
import { draftText, recycleMark, recycleSaving, scrollArmed, scrollRestored } from "./lib/recycle.mjs";
import { agentProcessesCountedAsApp, allocatorRoots, dumpedProcesses, FAKE_CLI_ROOTS, lastSample, mainPage, withChromiumRoles } from "./lib/sample.mjs";
import { agentsWithoutTurns, compare, compareRuns, comparisonTable, cpuTable, runsTable, wallSummary, groupOf, groupSample, inMegabytes, median, MB, steady, table, transcriptGrowthProblem } from "./lib/summary.mjs";

const proc = (pid, name, role, app, privateMB, commitMB = privateMB * 2) => ({ pid, name, role, app, private: privateMB * MB, commit: commitMB * MB });

const SAMPLE = {
    procs: [
        proc(1, "agentmux.exe", "main", true, 4),
        proc(2, "agentmux-0.59.30.exe", "main", true, 30),
        proc(3, "agentmux-srv-0.59.30-windows.x64.exe", "main", true, 110),
        proc(4, "agentmux-0.59.30.exe", "gpu-process", true, 160),
        proc(5, "agentmux-0.59.30.exe", "renderer", true, 700),
        proc(6, "agentmux-0.59.30.exe", "renderer", true, 50),
        proc(7, "agentmux-0.59.30.exe", "utility", true, 10),
        proc(8, "agentmux-0.59.30.exe", "crashpad-handler", true, 2),
        proc(9, "claude.exe", "main", false, 220),
        proc(10, "bash.exe", "main", false, 2),
    ],
};

describe("memory lab summary", () => {
    it("puts a process in the group the budget counts it under", () => {
        expect(groupOf(SAMPLE.procs[0])).toBe("host");
        expect(groupOf(SAMPLE.procs[2])).toBe("srv");
        expect(groupOf(SAMPLE.procs[3])).toBe("gpu");
        expect(groupOf(SAMPLE.procs[4])).toBe("renderer");
        expect(groupOf(SAMPLE.procs[6])).toBe("utility");
        expect(groupOf(SAMPLE.procs[7])).toBe("other");
    });

    it("totals the app's own processes and counts the panes' children apart", () => {
        const g = groupSample(SAMPLE);
        expect(g.groups.renderer.private).toBe(750 * MB);
        expect(g.groups.renderer.procs).toBe(2);
        expect(g.groups.host.private).toBe(34 * MB);
        expect(g.app.private).toBe((4 + 30 + 110 + 160 + 750 + 10 + 2) * MB);
        expect(g.panes.private).toBe(222 * MB);
        expect(g.panes.procs).toBe(2);
    });

    it("takes the median, so one odd sample doesn't decide the steady value", () => {
        expect(median([3, 1, 2])).toBe(2);
        expect(median([4, 1, 2, 3])).toBe(2.5);
        expect(median([])).toBeNull();
        const odd = { procs: [proc(5, "agentmux.exe", "renderer", true, 5000)] };
        const calm = (mb) => ({ procs: [proc(5, "agentmux.exe", "renderer", true, mb)] });
        const s = steady([calm(700), odd, calm(710)]);
        expect(s.groups.renderer.private).toBe(710 * MB);
        expect(s.samples).toBe(3);
    });

    it("compares two runs per group, the biggest increase first", () => {
        const base = inMegabytes(steady([SAMPLE]));
        const lessRenderer = { procs: SAMPLE.procs.map((p) => (p.pid === 5 ? { ...p, private: 400 * MB } : p.pid === 4 ? { ...p, private: 180 * MB } : p)) };
        const head = inMegabytes(steady([lessRenderer]));
        const cmp = compare(base, head);
        expect(cmp.groups[0].name).toBe("gpu");
        expect(cmp.groups[0].deltaMB).toBe(20);
        const renderer = cmp.groups.find((r) => r.name === "renderer");
        expect(renderer.deltaMB).toBe(-300);
        expect(renderer.pct).toBe(-40);
        expect(cmp.app.deltaMB).toBe(-280);
        expect(comparisonTable(cmp, "main", "branch")).toMatch(/\| renderer \| 750 \| 450 \| -300 \| -40% \|/);
    });

    it("lists the groups that have processes, and the totals, in the table", () => {
        const md = table(inMegabytes(steady([SAMPLE])), "0.59.30");
        expect(md).toMatch(/^### 0\.59\.30/);
        expect(md).toMatch(/\| renderer \| 2 \| 750 \| 1500 \|/);
        expect(md).toMatch(/\*\*app total\*\*/);
        expect(md).not.toMatch(/\| other \| 0 /);
    });

    it("doesn't sum a missing metric as zero: the group's total is unknown, and so is its change", () => {
        const sample = { procs: [proc(1, "a.exe", "renderer", true, 400), { ...proc(2, "a.exe", "renderer", true, 0), private: null }, proc(3, "a.exe", "gpu-process", true, 90)] };
        const g = groupSample(sample);
        expect(g.groups.renderer.private).toBeNull();
        expect(g.groups.gpu.private).toBe(90 * MB);
        expect(g.app.private).toBeNull();
        // a steady value skips the samples that lack it
        const whole = { procs: [proc(1, "a.exe", "renderer", true, 420)] };
        expect(steady([sample, whole, whole]).groups.renderer.private).toBe(420 * MB);
        const unknown = inMegabytes(steady([sample]));
        expect(unknown.groups.renderer.privateMB).toBeNull();
        const cmp = compare(unknown, inMegabytes(steady([whole])));
        expect(cmp.groups.find((r) => r.name === "renderer").deltaMB).toBeNull();
        expect(comparisonTable(cmp)).toMatch(/\| renderer \| null \| 420 \| n\/a \| - \|/);
    });

    it("refuses to compare runs of a different layout, length or conversation", () => {
        const run = { layout: { panes: 4, tabs: [4] }, rounds: 10, seed: 1 };
        expect(mismatch({ ...run, settleSeconds: 20, steady: { samples: 3 } }, { ...run, settleSeconds: 5, steady: { samples: 3 } })).toMatch(/different settle delays/);
        expect(mismatch({ ...run, settleSeconds: 20, steady: { samples: 3 } }, { ...run, settleSeconds: 20, steady: { samples: 1 } })).toMatch(/different sample counts/);
        expect(mismatch(run, { ...run })).toBeNull();
        expect(mismatch(run, { ...run, layout: { panes: 12, tabs: [4, 2, 4, 2] } })).toMatch(/different layouts/);
        expect(mismatch(run, { ...run, rounds: 20 })).toMatch(/different rounds/);
        expect(mismatch(run, { ...run, seed: 2 })).toMatch(/different conversations/);
        expect(mismatch({ ...run, gapSeconds: 15, platform: "win32" }, { ...run, gapSeconds: 5, platform: "win32" })).toMatch(/different round spacing/);
        const same = { ...run, settleSeconds: 20, steady: { samples: 3 }, gapSeconds: 15, platform: "win32" };
        expect(mismatch({ ...same, recycleAfter: 3, recycleCommand: "R" }, { ...same, recycleAfter: 0 })).toMatch(/different recycles/);
        expect(mismatch({ ...same, recycleAfter: 3, recycleCommand: "R" }, { ...same })).toMatch(/different recycles/);
        expect(mismatch({ ...same, recycleAfter: 3, recycleCommand: "R" }, { ...same, recycleAfter: 5, recycleCommand: "R" })).toMatch(/different recycles/);
        expect(mismatch({ ...same, recycleAfter: 3, recycleCommand: "R" }, { ...same, recycleAfter: 3, recycleCommand: "S" })).toMatch(/different recycle commands/);
        const did = (recycled, blocked) => ({ ...same, recycleAfter: 3, recycleCommand: "R", recycle: { recycled, blocked } });
        expect(mismatch({ ...did(true), label: "main" }, { ...did(false, "no notice"), label: "branch" })).toMatch(/only one of the runs recycled its window: branch did not \(no notice\)/);
        expect(mismatch(did(true), did(true))).toBeNull();
        expect(mismatch(did(false, "a"), did(false, "b"))).toBeNull();
        expect(mismatch({ ...same }, { ...same, recycleAfter: 0, recycleCommand: null })).toBeNull();
        const load = (rateMBps, capGB) => ({ ...same, diskLoad: { rateMBps, capGB } });
        expect(mismatch(load(50, 20), load(50, 20))).toBeNull();
        expect(mismatch(load(50, 20), same)).toMatch(/different background disk loads/);
        expect(mismatch(load(50, 20), load(100, 20))).toMatch(/different background disk loads/);
        expect(mismatch(load(50, 20), load(50, 5))).toMatch(/capped at 20 GB vs .*capped at 5 GB/);
        expect(mismatch({ ...same, holdMinutes: 10 }, { ...same, holdMinutes: 0 })).toMatch(/different idle holds/);
        expect(mismatch({ ...same, holdMinutes: 10 }, { ...same, holdMinutes: 10 })).toBeNull();
        expect(mismatch({ ...run, gapSeconds: 15, platform: "win32" }, { ...run, gapSeconds: 15, platform: "linux" })).toMatch(/different operating systems/);
    });
});

describe("memory lab sampling", () => {
    const events = [
        { ph: "M", name: "process_name", pid: 11, args: { name: "Renderer" } },
        { ph: "M", name: "process_labels", pid: 11, args: { labels: "AgentMux" } },
        {
            ph: "v",
            pid: 11,
            args: {
                dumps: {
                    process_totals: { private_footprint_bytes: "1f4" },
                    allocators: {
                        v8: { attrs: { size: { value: "64" } } },
                        "malloc/partitions": { attrs: { size: { value: "a" } } },
                        "malloc/partitions/x": { attrs: { size: { value: "5" } } },
                        "malloc/other": { attrs: { size: { value: "7" } } },
                    },
                },
            },
        },
        { ph: "X", pid: 99 },
    ];

    it("reads each process's name, labels, totals and allocators from a memory-infra trace", () => {
        const [p] = dumpedProcesses(events);
        expect(p.pid).toBe(11);
        expect(p.name).toBe("Renderer");
        expect(p.labels).toBe("AgentMux");
        expect(p.totals.private_footprint_bytes).toBe(500);
        expect(p.alloc.v8.size).toBe(100);
    });

    it("reduces allocators to their roots: the root's own size, else the sum of its direct children", () => {
        const [p] = dumpedProcesses(events);
        const roots = allocatorRoots({ ...p.alloc, "malloc/partitions": { size: null } });
        expect(roots.v8).toBe(100);
        expect(roots.malloc).toBe(7);
        expect(allocatorRoots({ malloc: { size: 40 }, "malloc/a": { size: 5 } }).malloc).toBe(40);
    });
});

describe("memory lab OS samples", () => {
    it("takes the last record that has processes, not a trailing macOS footprint record", () => {
        const text = [{ run: { os: "macos" } }, { procs: [{ pid: 1 }] }, { procs: [{ pid: 2 }] }, { footprint: { total: 5 } }].map((r) => JSON.stringify(r)).join("\n") + "\n";
        expect(lastSample(text).procs[0].pid).toBe(2);
        expect(() => lastSample(JSON.stringify({ run: {} }))).toThrow(/no sample/);
    });
});

describe("memory lab validity guards", () => {
    it("names the agents whose transcript shows no played turn", () => {
        const rows = [{ name: "t1-1", tools: 14 }, { name: "t1-2", tools: 0 }, { name: "t1-3", tools: 6 }, { name: "t1-4", tools: 3 }];
        expect(agentsWithoutTurns(rows)).toEqual(["t1-2", "t1-4"]);
        expect(agentsWithoutTurns(rows.slice(0, 1))).toEqual([]);
        expect(agentsWithoutTurns([{ name: "x", tools: undefined }])).toEqual(["x"]);
    });

    const base = { before: 2 * MB, agents: 4, rounds: 10, bytesPerTurn: 145_000 };
    it("accepts a transcript store that grew by about what the played turns weigh", () => {
        expect(transcriptGrowthProblem({ ...base, after: 2 * MB + 4 * 10 * 145_000 })).toBeNull();
        expect(transcriptGrowthProblem({ ...base, after: 2 * MB + 4 * 10 * 145_000 * 0.85 })).toBeNull();
    });

    it("refuses a store that barely grew: the agents never played", () => {
        expect(transcriptGrowthProblem({ ...base, after: 2 * MB + 30_000 })).toMatch(/grew by 0\.0 MB; 4 agents playing 10 turns should add at least 4\.4 MB/);
    });

    it("keeps macOS's footprint record beside the last sample", () => {
        const text = [{ procs: [{ pid: 1 }] }, { footprint: { output: "total 1 GB" } }].map((r) => JSON.stringify(r)).join("\n");
        const s = lastSample(text);
        expect(s.procs[0].pid).toBe(1);
        expect(s.footprint.output).toBe("total 1 GB");
        expect(lastSample(JSON.stringify({ procs: [] })).footprint).toBeUndefined();
    });
});

describe("memory lab recycle check", () => {
    it("counts a scroll position as restored when the same row is at the top, about where it was", () => {
        const before = { scrollTop: 900, range: 1500, node: "tool_42", offset: -12 };
        expect(scrollRestored(before, { ...before, offset: 10 })).toBe(true);
        expect(scrollRestored(before, { ...before, offset: 80 })).toBe(false);
        expect(scrollRestored(before, { ...before, node: "tool_43" })).toBe(false);
        expect(scrollRestored(before, null)).toBe(false);
        expect(scrollRestored({ ...before, node: null }, { ...before, node: null })).toBe(false);
    });

    it("only counts a scroll as armed when the pane really is above the end of its transcript", () => {
        expect(scrollArmed({ scrollTop: 8471, range: 9071, node: "n", offset: 0 })).toBe(true);
        expect(scrollArmed({ scrollTop: 8960, range: 9071, node: "n", offset: 0 })).toBe(false);
        expect(scrollArmed({ scrollTop: 0, range: 0, node: "n", offset: 0 })).toBe(false);
        expect(scrollArmed(null)).toBe(false);
    });

    it("makes a different draft each run, so a stale one can't pass", () => {
        expect(draftText("a")).not.toBe(draftText("b"));
        expect(draftText("a")).toMatch(/^lab-draft-a:/);
    });

    const sample = (rendererMB, documents) => ({
        procs: [proc(1, "agentmux.exe", "renderer", true, rendererMB), proc(2, "agentmux.exe", "renderer", true, 50), proc(3, "agentmux-srv.exe", "main", true, 100)],
        pages: [
            { id: "x", url: "http://127.0.0.1:5000/index.html", dom: { documents, nodes: 30000, jsEventListeners: 900 }, jsHeapUsed: 80 * MB, layoutObjects: 12000, probe: { toolBlocks: 40, toolTextKB: 2048 } },
            { id: "pool", url: "http://localhost/?windowLabel=window-pool-1", dom: { documents: 9 } },
        ],
    });

    it("reads the app and renderer totals and the main window's document count from a sample", () => {
        const m = recycleMark(sample(700, 1), 30);
        expect(m).toEqual({
            atSeconds: 30, appMB: 850, rendererMB: 750, rendererProcs: 2, documents: 1, pages: 2,
            nodes: 30000, jsHeapMB: 80, layoutObjects: 12000, listeners: 900, toolBlocks: 40, toolTextKB: 2048,
        });
    });

    it("measures the saving from the settled mark against the mark before, and none for a refused recycle", () => {
        const rec = { recycled: true, before: recycleMark(sample(700, 1), 0), after: [recycleMark(sample(500, 2), 5), recycleMark(sample(300, 1), 60)], state: { pass: true } };
        expect(recycleSaving(rec)).toEqual({ appMB: 400, rendererMB: 400 });
        expect(recycleSaving({ recycled: false, blocked: "a file is still uploading" })).toBeNull();
        expect(recycleSaving(null)).toBeNull();
    });
});

describe("memory lab packaged builds", () => {
    const has = (...present) => (p) => present.some((x) => p.replace(/\\/g, "/").endsWith(x));
    it("finds each platform's executable in what `task package` makes", () => {
        expect(portableExe("C:/p/agentmux-0.59.32-x64-portable", "win32")).toMatch(/agentmux\.exe$/);
        expect(portableExe("/p/dist", "darwin", has("AgentMux.app")).replace(/\\/g, "/")).toBe("/p/dist/AgentMux.app/Contents/MacOS/agentmux-launcher");
        expect(portableExe("/p/AgentMux.app", "darwin", has()).replace(/\\/g, "/")).toBe("/p/AgentMux.app/Contents/MacOS/agentmux-launcher");
        expect(portableExe("/p/agentmux-linux", "linux", has("agentmux.sh")).replace(/\\/g, "/")).toBe("/p/agentmux-linux/agentmux.sh");
        expect(portableExe("/p/agentmux-linux", "linux", has()).replace(/\\/g, "/")).toBe("/p/agentmux-linux/agentmux");
    });
});

describe("memory lab Linux roles", () => {
    it("gives a zygote-forked process the type Chromium reports for it", () => {
        const sample = { procs: [proc(10, "agentmux", "zygote-forked", true, 400), proc(11, "agentmux", "zygote-forked", true, 90), proc(12, "agentmux", "zygote", true, 5), proc(13, "agentmux", "renderer", true, 1)] };
        const fixed = withChromiumRoles(sample, [{ id: 10, type: "renderer" }, { id: 11, type: "gpu" }, { id: 13, type: "renderer" }]);
        expect(fixed.procs.map((p) => p.role)).toEqual(["renderer", "gpu-process", "zygote", "renderer"]);
        expect(groupSample(fixed).groups.renderer.private).toBe(401 * MB);
        expect(groupSample(fixed).groups.gpu.private).toBe(90 * MB);
    });
});

describe("memory lab pages", () => {
    it("finds the main window's page, not a pool window or a browser pane", () => {
        const pages = [
            { id: "a", url: "http://localhost:5173/?windowLabel=window-pool-1&pool=1" },
            { id: "b", url: "https://example.com/" },
            { id: "c", url: "http://127.0.0.1:55001/index.html" },
        ];
        expect(mainPage(pages).id).toBe("c");
        expect(mainPage(pages.slice(0, 2))).toBeNull();
    });
});

describe("memory lab conversation", () => {
    it("is repeatable from its seed and shaped like a tool-heavy turn", () => {
        const a = generateConversation({ turns: 3, seed: 5 });
        expect(a).toEqual(generateConversation({ turns: 3, seed: 5 }));
        expect(a).not.toEqual(generateConversation({ turns: 3, seed: 6 }));
        expect(a.turns).toHaveLength(3);
        const tools = a.turns[0].steps.filter((s) => s.tool);
        expect(tools.length).toBeGreaterThanOrEqual(8);
        expect(tools.length).toBeLessThanOrEqual(20);
        expect(a.turns[0].steps.at(-1).text.length).toBeGreaterThan(500);
    });
});

describe("memory soak gate", () => {
    const workload = { platform: "win32", panes: 12, rounds: 10, seed: 1, recycleAfter: 0 };
    const result = (app, renderer, over = {}) => ({
        platform: "win32", layout: { panes: 12 }, rounds: 10, seed: 1, recycleAfter: 0,
        steady: { samples: 3, app: { privateMB: app }, groups: { renderer: { privateMB: renderer }, srv: { privateMB: 90 } } },
        ...over,
    });
    const budget = (mode, limitsMB) => ({ mode, workload, limitsMB });

    it("cannot fail while no line is set", () => {
        const out = evaluate(result(5000, 4000), budget("enforce", { app: null, renderer: null, srv: null }));
        expect(out.over).toBe(false);
        expect(out.exitCode).toBe(0);
    });

    it("reports what would fail but exits 0 in report mode", () => {
        const out = evaluate(result(1100, 700), budget("report", { app: 1000, renderer: 800, srv: null }));
        expect(out.over).toBe(true);
        expect(out.exitCode).toBe(0);
        expect(report(out, result(1100, 700), budget("report", { app: 1000 }))).toMatch(/would be over/);
    });

    it("exits 1 over a line in enforce mode and 0 within it", () => {
        expect(evaluate(result(1100, 700), budget("enforce", { app: 1000 })).exitCode).toBe(1);
        expect(evaluate(result(999, 700), budget("enforce", { app: 1000 })).exitCode).toBe(0);
        expect(evaluate(result(1000, 700), budget("enforce", { app: 1000 })).exitCode).toBe(0);
    });

    it("does not pass a result that is not the gated workload", () => {
        const out = evaluate(result(500, 300, { layout: { panes: 4 } }), budget("enforce", { app: 1000 }));
        expect(out.problem).toMatch(/panes/);
        expect(out.exitCode).toBe(2);
        expect(evaluate(result(500, 300, { recycleAfter: 5 }), budget("report", {})).problem).toMatch(/recycleAfter/);
    });

    it("does not pass a line whose number could not be read", () => {
        const r = result(900, 600);
        delete r.steady.groups.srv;
        const out = evaluate(r, budget("enforce", { srv: 120 }));
        expect(out.over).toBe(true);
        expect(out.exitCode).toBe(2);
        expect(report(out, r, budget("enforce", { srv: 120 }))).toMatch(/\| srv \| n\/a MB \| 120 MB \| not read \|/);
    });
});

describe("replacing a portable's srv", () => {
    it("finds the one srv file wherever it sits", () => {
        expect(srvTarget(["agentmux.exe", "runtime/agentmux-srv-0.59.32-windows.x64.exe", "README.txt"])).toBe("runtime/agentmux-srv-0.59.32-windows.x64.exe");
        expect(srvTarget(["AgentMux.app/Contents/MacOS/agentmux-srv", "AgentMux.app/Contents/MacOS/agentmux-launcher"])).toBe("AgentMux.app/Contents/MacOS/agentmux-srv");
    });

    it("refuses a portable with no srv or two", () => {
        expect(() => srvTarget(["agentmux.exe"])).toThrow(/found 0/);
        expect(() => srvTarget(["a/agentmux-srv-1.exe", "b/agentmux-srv-2.exe"])).toThrow(/found 2/);
    });

    it("copies the portable, swaps the srv in the copy and leaves the original alone", () => {
        const root = mkdtempSync(join(tmpdir(), "lab-srv-"));
        try {
            const portable = join(root, "portable");
            mkdirSync(join(portable, "runtime"), { recursive: true });
            writeFileSync(join(portable, "agentmux.exe"), "host");
            writeFileSync(join(portable, "runtime", "agentmux-srv-1.0.exe"), "old srv");
            const mine = join(root, "my-srv.exe");
            writeFileSync(mine, "new srv");
            const out = withReplacedSrv(portable, mine, join(root, "out"));
            expect(basename(out.portable)).toBe("portable");
            expect(readFileSync(join(out.portable, "runtime", "agentmux-srv-1.0.exe"), "utf8")).toBe("new srv");
            expect(readFileSync(join(out.portable, "agentmux.exe"), "utf8")).toBe("host");
            expect(readFileSync(join(portable, "runtime", "agentmux-srv-1.0.exe"), "utf8")).toBe("old srv");
            expect(out.replaced).toMatchObject({ file: join("runtime", "agentmux-srv-1.0.exe"), bytes: 7 });
            expect(() => withReplacedSrv(portable, join(root, "missing.exe"), join(root, "out"))).toThrow(/not a file/);
        } finally {
            rmSync(root, { recursive: true, force: true });
        }
    });
});

describe("CPU and round time", () => {
    const proc = (pid, role, cpu_ns, name = "x.exe") => ({ pid, name, role, app: true, cpu_ns });

    it("counts the highest CPU time seen per process, so a replaced renderer keeps what it spent", () => {
        const ledger = new CpuLedger();
        ledger.note([proc(1, "renderer", 2e9), proc(2, "main", 1e9, "agentmux-srv.exe")]);
        const atStart = ledger.totals();
        ledger.note([proc(1, "renderer", 5e9), proc(2, "main", 4e9, "agentmux-srv.exe")]);
        // the renderer is recycled: pid 1 is gone, pid 3 is new and has used 1 s
        ledger.note([proc(3, "renderer", 1e9), proc(2, "main", 6e9, "agentmux-srv.exe")]);
        const spent = cpuBetween(atStart, ledger.totals());
        expect(spent.renderer).toBe(4); // 5 - 2 for pid 1, plus 1 for pid 3
        expect(spent.srv).toBe(5);
        expect(spent.app).toBe(9);
    });

    it("ignores the panes' children and processes whose CPU could not be read", () => {
        const ledger = new CpuLedger();
        ledger.note([{ ...proc(1, "renderer", 9e9), app: false }, proc(2, "renderer", null)]);
        expect(ledger.totals().app).toBe(0);
    });

    it("summarises round times and says how many could not be timed", () => {
        const w = wallSummary([12000, null, 8000, 10000]);
        expect(w).toMatchObject({ medianMs: 10000, maxMs: 12000, timedRounds: 3, untimedRounds: 1 });
        expect(wallSummary([null, null]).medianMs).toBeNull();
        expect(cpuTable({ seconds: { renderer: 30, srv: 5, app: 40 } }, w)).toMatch(/1 of 4 rounds not timed/);
    });

    const run = (label, app, renderer, cpuApp, cpuRenderer, roundMs) => ({
        label,
        steady: { app: { privateMB: app }, groups: { renderer: { privateMB: renderer } } },
        cpu: { seconds: { app: cpuApp, renderer: cpuRenderer } },
        wall: { medianMs: roundMs },
    });

    it("compares the medians of several runs and flags CPU or round time past the margin", () => {
        const base = [run("b1", 900, 600, 100, 70, 10000), run("b2", 920, 610, 104, 72, 10400), run("b3", 910, 605, 102, 71, 10200)];
        const head = [run("h1", 700, 450, 125, 90, 10100), run("h2", 710, 455, 127, 92, 10300)];
        const cmp = compareRuns(base, head);
        const app = cmp.rows.find((r) => r.group === "app");
        expect(app.memory.pct).toBeLessThan(-20);
        expect(app.cpu.flagged).toBe(true); // 102 -> 126, about +23%, past +10%, and the ranges are apart
        expect(cmp.rows.find((r) => r.group === "renderer").cpu.flagged).toBe(true); // 71 -> 91, about +28%, past +20%
        expect(cmp.wall.flagged).toBe(false);
        const text = runsTable(cmp, "base", "head");
        expect(text).toMatch(/CPU over \+10%/);
        expect(text).toMatch(/CPU over \+20%/);
        expect(text).toMatch(/3 run\(s\) against 2/);
    });

    it("does not flag a change inside the margin, and treats a missing number as unknown", () => {
        const cmp = compareRuns([run("b", 900, 600, 100, 70, 10000)], [run("h", 800, 500, 105, 72, 10500)]);
        expect(cmp.rows.find((r) => r.group === "app").cpu.flagged).toBe(false);
        const old = compareRuns([{ label: "o", steady: { app: { privateMB: 900 }, groups: {} } }], [run("h", 800, 500, 105, 72, 10500)]);
        expect(old.rows.find((r) => r.group === "app").cpu.base.n).toBe(0);
        expect(old.rows.find((r) => r.group === "app").cpu.flagged).toBe(false);
    });

    it("applies the margin per metric and needs the ranges apart", () => {
        // renderer CPU +15%: past 10 but not past its own 20; app CPU +15%: past 10
        const base = [run("b1", 900, 600, 100, 70, 10000), run("b2", 900, 600, 101, 70.5, 10000)];
        const head = [run("h1", 900, 600, 116, 81, 10000), run("h2", 900, 600, 117, 81.5, 10000)];
        const cmp = compareRuns(base, head);
        expect(cmp.rows.find((r) => r.group === "app").cpu.flagged).toBe(true);
        expect(cmp.rows.find((r) => r.group === "renderer").cpu.flagged).toBe(false);
        // the same +15% app CPU with overlapping ranges is noise, not a flag
        const noisy = compareRuns([run("b1", 900, 600, 100, 70, 10000), run("b2", 900, 600, 120, 70, 10000)], [run("h1", 900, 600, 118, 70, 10000), run("h2", 900, 600, 140, 70, 10000)]);
        expect(noisy.rows.find((r) => r.group === "app").cpu.pct).toBeGreaterThan(10);
        expect(noisy.rows.find((r) => r.group === "app").cpu.flagged).toBe(false);
        // a single number sets every CPU group
        expect(compareRuns(base, head, { cpuPct: 10, wallPct: 20 }).rows.find((r) => r.group === "renderer").cpu.flagged).toBe(true);
    });

    it("flags a run with a page command over 10 s", () => {
        const a = { ...run("a", 900, 600, 100, 70, 10000), slowestEvalMs: 2100 };
        const b = { ...run("b", 900, 600, 100, 70, 10000), slowestEvalMs: 14300 };
        const cmp = compareRuns([a], [b]);
        expect(cmp.slow).toEqual([{ side: "head", label: "b", ms: 14300 }]);
        expect(runsTable(cmp, "base", "head")).toMatch(/SLOW PAGE COMMAND: b \(head\).*14\.3 s/);
        expect(compareRuns([a], [a]).slow).toEqual([]);
        expect(compareRuns([run("old", 900, 600, 1, 1, 1)], [a]).slow).toEqual([]); // no figure: not flagged
    });

    it("refuses to compare different workload versions", () => {
        const same = { layout: { panes: 4 }, rounds: 10, seed: 1, gapSeconds: 15, platform: "win32", settleSeconds: 20, steady: { samples: 3 } };
        expect(mismatch({ ...same, workloadVersion: 1 }, { ...same, workloadVersion: 2 })).toMatch(/workload versions/);
        expect(mismatch({ ...same }, { ...same, workloadVersion: 1 })).toBeNull(); // no stamp is version 1
        expect(mismatch({ ...same, workloadVersion: 2 }, { ...same, workloadVersion: 2 })).toBeNull();
    });
});

describe("Linux process roles", () => {
    // The roles of a real 12-pane Linux run (Ubuntu, CEF): the CDP step names the GPU process "GPU" and a utility process by its service.
    const role = (role, name = "agentmux-cef") => groupOf({ role, name, app: true });

    it("groups Chromium's Linux roles as the Windows ones", () => {
        expect(role("renderer")).toBe("renderer");
        expect(role("GPU")).toBe("gpu");
        expect(role("gpu-process")).toBe("gpu");
        expect(role("utility")).toBe("utility");
        expect(role("storage.mojom.StorageService")).toBe("utility");
        expect(role("tracing.mojom.TracingService")).toBe("utility");
        expect(role("zygote")).toBe("utility");
        expect(role("zygote-forked")).toBe("utility");
        expect(role("main", "agentmux-launcher")).toBe("host");
        expect(role("main", "agentmux-srv")).toBe("srv");
        expect(role("crashpad-handler")).toBe("other");
    });
});

describe("an agent's process is never the app", () => {
    const proc = (name, app) => ({ pid: 1, name, role: "main", app });

    it("names the agent CLI's process on each platform among the panes' roots", () => {
        const roots = FAKE_CLI_ROOTS.split(",");
        expect(roots).toContain("node"); // Windows (node.exe is matched without its extension) and macOS
        expect(roots).toContain("MainThread"); // Linux: a node process's name reads MainThread
    });

    it("finds an agent CLI that was counted as the app, by any of those names", () => {
        const counted = agentProcessesCountedAsApp([proc("node.exe", true), proc("MainThread", true), proc("node", true), proc("claude", true)]);
        expect(counted).toHaveLength(4);
    });

    it("leaves the app's own processes and the panes' children alone", () => {
        expect(agentProcessesCountedAsApp([proc("agentmux.exe", true), proc("agentmux-cef", true), proc("agentmux-srv", true), proc("node.exe", false), proc("MainThread", false)])).toEqual([]);
        expect(agentProcessesCountedAsApp(undefined)).toEqual([]);
    });
});

describe("naming the driver's commands", () => {
    it("names an RPC by its command, and anything else by its first characters", () => {
        expect(commandName("RpcApi.AgentInputCommand(TabRpcClient, { blockid: 'x' })")).toBe("AgentInput");
        expect(commandName('TabRpcClient.rpcCall("agent.open", {"agent_id":"a"})')).toBe("agent.open");
        expect(commandName('TabRpcClient.rpcCall("createagent", {})')).toBe("createagent");
        expect(commandName("document.querySelectorAll('.agent-working-row').length")).toMatch(/^document\.querySelectorAll/);
    });
});

describe("srv drift", () => {
    it("reads the commit from a portable's name", () => {
        expect(portableCommit("C:/b/agentmux-0.59.32+gfe727c686.20261018T111038.865-x64-portable")).toBe("fe727c686");
        expect(portableCommit("C:/b/some-folder")).toBeNull();
    });

    it("warns when the two commits are known and differ, not when they agree or one is unknown", () => {
        const p = "C:/b/agentmux-0.59.32+gfe727c686.2026-x64-portable";
        expect(driftWarning(p, "7c1730e0e")).toMatch(/may have drifted/);
        expect(driftWarning(p, "fe727c686")).toBeNull();
        expect(driftWarning(p, "fe727c686a1b2c3d4e5f")).toBeNull(); // a longer form of the same commit
        expect(driftWarning(p, null)).toBeNull();
        expect(driftWarning("C:/b/plain", "7c1730e0e")).toBeNull();
    });
});

describe("the store lock in srv's log", () => {
    const line = (message, fields) => JSON.stringify({ timestamp: "2026-10-19T08:30:12Z", level: "WARN", fields: { message, ...fields }, target: "agentmux_srv::backend::storage::filestore::core" });

    it("finds the longest hold, with where it was taken, and the longest wait", () => {
        const text = [
            line("filestore: connection held a long time", { site: "crates\\srv\\src\\backend\\agent_session\\helpers.rs:31:21", held_ms: 4707 }),
            line("filestore: connection held a long time", { site: "crates\\srv\\src\\backend\\blockcontroller\\shell\\file_ops.rs:291:16", held_ms: 9445 }),
            line("filestore: slow to get the connection", { site: "x", waited_ms: 3059 }),
            line("filestore: slow to get the connection", { site: "x", waited_ms: 9444 }),
            JSON.stringify({ fields: { message: "something else", held_ms: 99999 } }),
            "not json at all, but mentions filestore: ",
        ].join("\n");
        expect(parseStoreLock(text)).toEqual({ longestHeldMs: 9445, longestHeldSite: "backend/blockcontroller/shell/file_ops.rs:291:16", holds: 2, longestWaitedMs: 9444, waits: 2 });
    });

    it("reads a log with none as zero, and merges several logs", () => {
        expect(parseStoreLock(line("agentmuxsrv starting", {}))).toMatchObject({ longestHeldMs: 0, holds: 0, waits: 0 });
        const a = { longestHeldMs: 900, longestHeldSite: "a", holds: 1, longestWaitedMs: 100, waits: 1 };
        const b = { longestHeldMs: 5200, longestHeldSite: "b", holds: 2, longestWaitedMs: 50, waits: 3 };
        expect(mergeStoreLock([a, b])).toEqual({ longestHeldMs: 5200, longestHeldSite: "b", holds: 3, longestWaitedMs: 100, waits: 4 });
    });

    it("shows the longest hold of each side in the comparison, report only", () => {
        const run = (label, held) => ({ label, steady: { app: { privateMB: 900 }, groups: {} }, storeLock: { longestHeldMs: held } });
        const cmp = compareRuns([run("b1", 800), run("b2", 1200)], [run("h1", 6000), run("h2", 9000)]);
        expect(cmp.lock.head.median).toBe(7500);
        const text = runsTable(cmp, "base", "head");
        expect(text).toMatch(/Longest hold of srv's transcript store connection .*base 1000 ms.*head 7500 ms/);
        // runs from before this was recorded add nothing
        expect(runsTable(compareRuns([{ label: "o", steady: { app: { privateMB: 1 }, groups: {} } }], [{ label: "p", steady: { app: { privateMB: 1 }, groups: {} } }]), "a", "b")).not.toMatch(/Longest hold/);
    });
});

describe("bytes written", () => {
    const MiB = 1048576;
    const proc = (pid, role, write_bytes, name = "x.exe") => ({ pid, name, role, app: true, write_bytes });

    it("counts the highest write counter seen per process, per group, so a replaced process keeps what it wrote", () => {
        const ledger = new CpuLedger();
        ledger.note([proc(1, "main", 10 * MiB, "agentmux-srv.exe"), proc(2, "renderer", 1 * MiB)]);
        const atStart = ledger.writtenTotals();
        ledger.note([proc(1, "main", 50 * MiB, "agentmux-srv.exe"), proc(2, "renderer", 3 * MiB)]);
        ledger.note([proc(1, "main", 90 * MiB, "agentmux-srv.exe"), proc(3, "renderer", 2 * MiB)]); // renderer 2 replaced by 3
        const written = writtenBetween(atStart, ledger.writtenTotals());
        expect(written.srv).toBe(80);
        expect(written.renderer).toBe(4); // 3 - 1 for pid 2, plus 2 for pid 3
        expect(written.app).toBe(84);
    });

    it("counts a process that reuses a PID with the same name as another process", () => {
        const ledger = new CpuLedger();
        ledger.note([{ ...proc(7, "renderer", 30 * MiB), start_key: 111 }]);
        // the renderer exits and a new one starts under the same PID and name, then writes 5 MiB
        ledger.note([{ ...proc(7, "renderer", 5 * MiB), start_key: 222 }]);
        expect(writtenBetween({ app: 0, renderer: 0 }, ledger.writtenTotals()).renderer).toBe(35);
    });

    it("counts the bytes that reached storage separately, and has none where the OS has no such counter", () => {
        const MiB2 = 1048576;
        const p = (pid, write_bytes, storage_write_bytes, name = "agentmux-srv.exe") => ({ pid, name, role: "main", app: true, write_bytes, storage_write_bytes });
        const ledger = new CpuLedger();
        ledger.note([p(1, 100 * MiB2, 20 * MiB2)]);
        ledger.note([p(1, 900 * MiB2, 120 * MiB2)]);
        expect(ledger.writtenTotals().srv).toBe(900 * MiB2); // asked to write
        expect(ledger.storageTotals().srv).toBe(120 * MiB2); // reached storage
        expect(ledger.storageIncomplete()).toEqual([]);
        // a later truncate cancels writeback that was counted: the storage figure may fall, and the ledger follows it
        ledger.note([p(1, 950 * MiB2, 60 * MiB2)]);
        expect(ledger.storageTotals().srv).toBe(60 * MiB2);
        expect(ledger.writtenTotals().srv).toBe(950 * MiB2); // the write counter only grows
        // a process that truncated what another dirtied is below zero alone; the group's sum is what is clamped
        const mixed = new CpuLedger();
        mixed.note([p(1, 500 * MiB2, 80 * MiB2), p(2, 10 * MiB2, -30 * MiB2, "agentmux-srv-helper.exe")]);
        expect(mixed.storageTotals().srv).toBe(50 * MiB2);
        expect(writtenBetween({ srv: 0 }, mixed.storageTotals()).srv).toBe(50);
        expect(writtenBetween({ srv: 200 * MiB2 }, mixed.storageTotals()).srv).toBe(0); // never below zero in the report
        const windows = new CpuLedger(); // Windows: no storage counter at all
        windows.note([p(1, 100 * MiB2, undefined)]);
        expect(windows.writeIncomplete()).toEqual([]);
        expect(windows.storageIncomplete().sort()).toEqual(["app", "srv"]);
        expect(windows.sawStorageCounters()).toBe(false); // so a Windows run records no storage figure, not n/a everywhere
        expect(ledger.sawStorageCounters()).toBe(true);
    });

    it("leaves out the panes' children and a process whose counter could not be read", () => {
        const ledger = new CpuLedger();
        ledger.note([{ ...proc(1, "renderer", 900 * MiB), app: false }, proc(2, "renderer", undefined)]);
        expect(ledger.writtenTotals().app).toBe(0);
        expect(ledger.writeIncomplete().sort()).toEqual(["app", "renderer"]); // process 2 never gave a counter: no total for its group
        const fine = new CpuLedger();
        fine.note([proc(3, "renderer", 0)]);
        expect(fine.writeIncomplete()).toEqual([]);
    });

    it("says which groups lack a counter when one process in a group is readable and another is not", () => {
        const ledger = new CpuLedger();
        ledger.note([proc(1, "main", 10 * MiB, "agentmux-srv.exe"), proc(2, "main", undefined, "agentmux-srv.exe"), proc(3, "renderer", 5 * MiB)]);
        expect(ledger.writeIncomplete().sort()).toEqual(["app", "srv"]);
        // the counter of process 2 turns up on a later sample: the group is complete again
        ledger.note([proc(2, "main", 1 * MiB, "agentmux-srv.exe")]);
        expect(ledger.writeIncomplete()).toEqual([]);
    });

    it("shows what reached storage beside what was written", () => {
        const run = (label, srv, stor) => ({ label, steady: { app: { privateMB: 900 }, groups: {} }, io: { writtenMB: { srv }, storageWrittenMB: stor == null ? null : { srv: stor } } });
        const text = runsTable(compareRuns([run("b1", 5000, 1000)], [run("h1", 2000, 400)]), "base", "head");
        expect(text).toMatch(/reached storage.*base 1000 MB.*head 400 MB/);
        expect(runsTable(compareRuns([run("b1", 5000, null)], [run("h1", 2000, null)]), "base", "head")).not.toMatch(/reached storage/);
    });

    it("compares what srv wrote on each side, flagged past +10% when the ranges are apart", () => {
        const run = (label, srv) => ({ label, steady: { app: { privateMB: 900 }, groups: {} }, io: { writtenMB: { srv } } });
        const cmp = compareRuns([run("b1", 5300), run("b2", 5400)], [run("h1", 2100), run("h2", 2200)]);
        expect(cmp.written.pct).toBeLessThan(-50);
        expect(cmp.written.flagged).toBe(false); // less is not a regression
        const worse = compareRuns([run("b1", 2000), run("b2", 2100)], [run("h1", 2600), run("h2", 2700)]);
        expect(worse.written.flagged).toBe(true);
        expect(runsTable(worse, "base", "head")).toMatch(/Bytes srv wrote during the workload.*base 2050 MB.*head 2650 MB.*over \+10%/);
    });
});

describe("the page's long tasks", () => {
    const raw = { n: 5, tasks: [[1000, 80], [2100, 2400], [4000, 60], [9000, 700], [9500, 120]], top: [{ at: 2100, ms: 2400, name: "self", container: "window" }] };

    it("summarises them overall and per round", () => {
        const s = summariseLongTasks(raw, [{ round: 1, start: 0, end: 3000 }, { round: 2, start: 3000, end: 10000 }]);
        expect(s).toMatchObject({ count: 5, totalMs: 3360, maxMs: 2400, over1s: 1 });
        expect(s.byRound).toEqual([
            { round: 1, count: 2, totalMs: 2480, maxMs: 2400 },
            { round: 2, count: 3, totalMs: 880, maxMs: 700 },
        ]);
        expect(s.longest[0]).toMatchObject({ ms: 2400, container: "window", at: new Date(2100).toISOString() });
        expect(s.truncated).toBe(false);
        // the lab's own forced-GC sampling window covers the 2.4 s task at 2100: it is the instrument's, not the workload's
        const excl = summariseLongTasks(raw, [{ round: 1, start: 0, end: 3000 }], [{ start: 2000, end: 4500 }]);
        expect(excl).toMatchObject({ excludedTasks: 2, count: 3, maxMs: 700 }); // the 2.4 s task and the 60 ms one at 4000
        expect(excl.longest).toEqual([]);
        expect(summariseLongTasks({ ...raw, n: 9000 }, []).truncated).toBe(true); // more were counted than kept
        expect(summariseLongTasks(null)).toBeNull(); // never installed, or lost to a reload
    });

    it("summarises the page's timer lag, leaving out the lab's own forced-GC windows", () => {
        const withLag = { n: 0, tasks: [], top: [], lag: [[5000, 300], [9000, 1500], [20000, 4000]] };
        expect(summariseLongTasks(withLag, []).timerLag).toEqual({ count: 3, maxMs: 4000, over1s: 2 });
        // the 4 s late firing at 20,000 waited over 16,000-20,000: inside the forced-GC window
        expect(summariseLongTasks(withLag, [], [{ start: 15000, end: 21000 }]).timerLag).toEqual({ count: 2, maxMs: 1500, over1s: 1 });
        expect(summariseLongTasks({ n: 0, tasks: [], top: [] }, []).timerLag).toEqual({ count: 0, maxMs: 0, over1s: 0 }); // an old record has none
    });

    it("joins the records of the documents before and after a reload", () => {
        const before = { n: 2, tasks: [[1000, 90], [2000, 3000]], top: [{ at: 2000, ms: 3000, name: "self", container: "" }] };
        const after = { n: 1, tasks: [[9000, 400]], top: [{ at: 9000, ms: 400, name: "self", container: "" }] };
        const merged = mergeLongTaskRecords([before, null, after]);
        expect(merged).toMatchObject({ n: 3 });
        expect(merged.tasks).toHaveLength(3);
        expect(merged.top.map((t) => t.ms)).toEqual([3000, 400]);
        expect(summariseLongTasks(merged, [{ round: 1, start: 0, end: 5000 }, { round: 2, start: 5000, end: 10000 }]).byRound.map((r) => r.count)).toEqual([2, 1]);
        expect(mergeLongTaskRecords([null])).toBeNull();
    });

    it("says how many slow sends ran while a long task of 500 ms or more did", () => {
        const sends = [
            { start: 2000, ms: 6000 }, // overlaps the 2.4 s task at 2100
            { start: 20000, ms: 7000 }, // slow, nothing on the page
            { start: 9100, ms: 300 }, // fast
        ];
        expect(slowSendsAgainstLongTasks(sends, raw)).toEqual({ slowSends: 2, overlappingLongTask: 1 });
        expect(slowSendsAgainstLongTasks(sends, null)).toEqual({ slowSends: 2, overlappingLongTask: 0 });
    });
});

describe("the background disk load", () => {
    it("reads the requested rate", () => {
        expect(parseDiskLoad(undefined)).toBeNull();
        expect(parseDiskLoad("0")).toBeNull();
        expect(parseDiskLoad("50")).toBe(50);
        expect(parseDiskLoad("2.5")).toBe(2.5);
        expect(parseDiskLoad("max")).toBe(Infinity);
        expect(parseDiskLoad("MAX")).toBe(Infinity);
        expect(() => parseDiskLoad("fast")).toThrow(/positive number/);
        expect(() => parseDiskLoad("-5")).toThrow(/positive number/);
    });

    it("reports a writer that died on its own, and one that reached its cap", async () => {
        const dir = mkdtempSync(join(tmpdir(), "lab-diskload-"));
        try {
            const dying = startDiskLoad({ dir, mbps: 5 });
            await new Promise((r) => setTimeout(r, 800));
            process.kill(dying.pid, "SIGKILL");
            await new Promise((r) => setTimeout(r, 800));
            expect(dying.diedEarly()).toMatch(/exited on its own/);
            const report = await dying.stop();
            expect(report.diedEarly).toMatch(/volume may be full/);

            const capped = startDiskLoad({ dir, mbps: Infinity, capGB: 0.02 }); // 20 MB
            await new Promise((r) => setTimeout(r, 2500));
            const done = await capped.stop();
            expect(done.capReached).toBe(true);
            expect(done.diedEarly).toBeUndefined();
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    }, 30000);

    it("stops once and returns the same report when asked again", async () => {
        const dir = mkdtempSync(join(tmpdir(), "lab-diskload-"));
        try {
            const load = startDiskLoad({ dir, mbps: 5 });
            const first = await load.stop();
            expect(await load.stop()).toBe(first);
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    }, 20000);

    it("sleeps to hold the rate, and not at all when behind or unthrottled", () => {
        // 100 MiB written after 1 s at 50 MB/s is a second ahead of schedule
        expect(sleepNeededMs(100 * 1048576, 1000, 50)).toBe(1000);
        expect(sleepNeededMs(10 * 1048576, 1000, 50)).toBe(0); // behind
        expect(sleepNeededMs(1e12, 1, Infinity)).toBe(0);
    });

    it("writes to the folder while it runs, reports what it wrote and cleans up", async () => {
        const dir = mkdtempSync(join(tmpdir(), "lab-diskload-"));
        try {
            const load = startDiskLoad({ dir, mbps: 20 });
            await new Promise((r) => setTimeout(r, 3500));
            const result = await load.stop();
            expect(result.rateMBps).toBe(20);
            expect(result.writtenMB).toBeGreaterThan(8);
            expect(result.writtenMB).toBeLessThan(400);
            expect(existsSync(join(dir, "diskload"))).toBe(false);
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    }, 20000);
});

describe("the volume under the data folder", () => {
    it("reports total, free and percent free", () => {
        const v = volumeOf(tmpdir());
        expect(v.totalGB).toBeGreaterThan(0);
        expect(v.freeGB).toBeLessThanOrEqual(v.totalGB);
        expect(v.freePct).toBeGreaterThanOrEqual(0);
        expect(v.freePct).toBeLessThanOrEqual(100);
    });

    it("says nothing for a path that does not exist", () => {
        expect(volumeOf(join(tmpdir(), "no-such-folder-for-the-lab", "x"))).toBeNull();
    });
});

describe("where a slow send's time went", () => {
    it("wraps the call so the page reports its own clock and stopwatch", () => {
        const src = timedCall("RpcApi.X()");
        expect(src).toContain("await RpcApi.X()");
        expect(src).toContain("pageStart");
        expect(src).toContain("performance.now()");
    });

    it("splits a send into before, in the page, and after", () => {
        // the lab started at t=1000, JS ran in the page at 1400 for 28,000 ms, the lab had the answer at 30,000
        expect(segmentsOf({ start: 1000, ms: 29000, pageStart: 1400, pageMs: 28000 })).toEqual({ before: 400, inPage: 28000, after: 600 });
        expect(segmentsOf({ start: 1000, ms: 100, pageStart: null, pageMs: null })).toEqual({ before: null, inPage: null, after: null });
        // clock noise never gives a negative segment
        expect(segmentsOf({ start: 1000, ms: 50, pageStart: 990, pageMs: 80 })).toEqual({ before: 0, inPage: 80, after: 0 });
    });

    it("says which segment held the most time in each slow send", () => {
        const sends = [
            { start: 0, ms: 30000, pageStart: 300, pageMs: 29500 }, // in the page
            { start: 0, ms: 12000, pageStart: 9000, pageMs: 2800 }, // before the page ran it
            { start: 0, ms: 9000, pageStart: 100, pageMs: 200 }, // after the page was done
            { start: 0, ms: 6000, pageStart: null, pageMs: null }, // no page figures
            { start: 0, ms: 300, pageStart: 10, pageMs: 250 }, // not slow
        ];
        expect(slowSendBreakdown(sends)).toEqual({
            slowSends: 4,
            largestSegment: { before: 1, inPage: 1, after: 1, unknown: 1 },
            maxBeforeMs: 9000,
            maxInPageMs: 29500,
            maxAfterMs: 8700,
        });
    });
});

describe("what the page's websocket received", () => {
    // [bucket start (epoch ms), messages, bytes, biggest message]
    const ws = [[1000, 5, 5000, 2000], [1100, 3, 900, 400], [4000, 1, 80, 80], [4100, 2, 200, 150]];

    it("totals the traffic in a window and finds the longest silence", () => {
        expect(arrivalsBetween(ws, 1000, 5000)).toEqual({ messages: 11, bytes: 6180, biggestMessageBytes: 2000, longestSilenceMs: 2800 });
        // nothing arrived at all: the whole window is silence
        expect(arrivalsBetween(ws, 2000, 3500)).toEqual({ messages: 0, bytes: 0, biggestMessageBytes: 0, longestSilenceMs: 1500 });
        expect(arrivalsBetween(undefined, 0, 1000)).toEqual({ messages: 0, bytes: 0, biggestMessageBytes: 0, longestSilenceMs: 1000 });
    });

    it("reports what arrived during each slow send's time in the page", () => {
        const sends = [
            { round: 2, agent: 3, ms: 6000, pageStart: 1000, pageMs: 3900 }, // slow, traffic arrived
            { round: 2, agent: 4, ms: 300, pageStart: 1000, pageMs: 280 }, // fast
            { round: 3, agent: 0, ms: 7000, pageStart: null, pageMs: null }, // no page figures
        ];
        const out = arrivalsDuringSlowSends(ws, sends);
        expect(out).toHaveLength(1);
        expect(out[0]).toMatchObject({ round: 2, agent: 3, inPageMs: 3900, messages: 11, longestSilenceMs: 2800 });
        expect(arrivalTotals(ws)).toEqual({ messages: 11, bytes: 6180, biggestMessageBytes: 2000 });
    });
});

describe("the measures the nightly performance check watches", () => {
    const run = (app, cpuApp, round, written) => ({
        layout: { panes: 12 },
        rounds: 10,
        steady: { app: { privateMB: app }, groups: { renderer: { privateMB: app * 0.7 }, srv: { privateMB: 40 } } },
        cpu: { seconds: { app: cpuApp, srv: 60, renderer: 300, gpu: 20 } },
        wall: { medianMs: round },
        io: written == null ? null : { writtenMB: { srv: written } },
        storeLock: { longestHeldMs: 0 },
        slowEvals: { total: 2 },
    });

    it("reads one run under the names of the history line", () => {
        expect(measuresOf(run(800, 400, 16000, 5000))).toEqual({
            app_mb: 800, renderer_mb: 560, srv_mb: 40, app_cpu_s: 400, srv_cpu_s: 60, renderer_cpu_s: 300, gpu_cpu_s: 20,
            round_ms: 16000, srv_written_mb: 5000, store_lock_max_ms: 0, slow_page_commands: 2,
        });
        // a run with no figure for a measure has null for it, never zero
        expect(measuresOf(run(800, 400, 16000, null)).srv_written_mb).toBeNull();
        expect(measuresOf({}).app_mb).toBeNull();
    });

    it("gives the lowest, median and highest of repeated runs and how wide that is", () => {
        expect(spreadOf([100, 110, 105])).toEqual({ n: 3, min: 100, median: 105, max: 110, widthPct: 9.5 });
        expect(spreadOf([null, undefined])).toBeNull();
        expect(spreadOf([50, null, 52])).toMatchObject({ n: 2, min: 50, max: 52 });
    });

    it("tabulates a build's runs, with the wall time of each", () => {
        const runs = [run(800, 400, 16000, 5000), run(820, 410, 17000, 5100), run(810, 405, 16500, 5050)];
        const t = spreadTable(runs);
        expect(t.app_mb).toMatchObject({ min: 800, median: 810, max: 820 });
        const md = spreadMarkdown(runs, [420, 431, 425]);
        expect(md).toMatch(/\| app_mb \| 3 \| 800 \| 810 \| 820 \|/);
        expect(md).toMatch(/420 s, 431 s, 425 s; median 425 s/);
    });
});

describe("the instance's environment", () => {
    it("reads NAME=value pairs, with the value as given", () => {
        expect(parseEnvPairs(["AGENTMUX_EXP_A=1", "OTHER=a=b", "EMPTY="])).toEqual({ AGENTMUX_EXP_A: "1", OTHER: "a=b", EMPTY: "" });
        expect(parseEnvPairs(undefined)).toEqual({});
        expect(() => parseEnvPairs(["nope"])).toThrow(/NAME=value/);
        expect(() => parseEnvPairs(["=x"])).toThrow(/NAME=value/);
        expect(() => parseEnvPairs(["1BAD=x"])).toThrow(/NAME=value/);
        for (const name of RESERVED) expect(() => parseEnvPairs([`${name}=x`])).toThrow(/sets that itself/);
    });

    it("drops the shell's AGENTMUX_* variables, adds the lab's own, then the --env ones, and records all three", () => {
        const shell = { PATH: "/bin", AGENTMUX_AGENT_ID: "me", AGENTMUX_HOME_OVERRIDE: "/mine", KEEP: "1" };
        const { env, record } = buildInstanceEnv(shell, { AGENTMUX_HOME_OVERRIDE: "/lab", AGENTMUX_CDP_PORT: "9333" }, { AGENTMUX_EXP_X: "on" });
        expect(env).toEqual({ PATH: "/bin", KEEP: "1", AGENTMUX_HOME_OVERRIDE: "/lab", AGENTMUX_CDP_PORT: "9333", AGENTMUX_EXP_X: "on" });
        expect(record).toEqual({ passed: { AGENTMUX_EXP_X: "on" }, labSet: ["AGENTMUX_CDP_PORT", "AGENTMUX_HOME_OVERRIDE"], droppedFromShell: ["AGENTMUX_AGENT_ID", "AGENTMUX_HOME_OVERRIDE"] });
    });

    it("compare.mjs reads the same variables in another order as one environment, notes a difference between sides and refuses one within a side", () => {
        const dir = mkdtempSync(join(tmpdir(), "lab-envcmp-"));
        try {
            const base = { label: "x", layout: { panes: 4, tabs: [4] }, rounds: 10, seed: 1, gapSeconds: 15, platform: "win32", settleSeconds: 20, steady: { samples: 3, app: { privateMB: 800 }, groups: {} } };
            const write = (name, passed) => {
                const path = join(dir, name);
                writeFileSync(path, JSON.stringify({ ...base, label: name, instanceEnv: { passed } }));
                return path;
            };
            const a = write("a", { B: "2", A: "1" });
            const b = write("b", { A: "1", B: "2" });
            const c = write("c", { A: "9" });
            const compare = (...args) => spawnSync(process.execPath, [join(process.cwd(), "scripts/perf/memory-lab/compare.mjs"), ...args], { encoding: "utf8" });
            expect(compare(a, b).stdout).not.toMatch(/Instance environment given/);
            expect(compare(a, c).stdout).toMatch(/Instance environment given with --env: a \{"A":"1","B":"2"\}, c \{"A":"9"\}/);
            const within = compare("--base", `${a},${c}`, "--head", b);
            expect(within.status).toBe(2);
            expect(within.stderr).toMatch(/base runs were given different --env values/);
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    });

    it("reaches a real child process", () => {
        const { env } = buildInstanceEnv(process.env, {}, { AGENTMUX_EXP_PROBE: "arrived" });
        const out = spawnSync(process.execPath, ["-e", "process.stdout.write(process.env.AGENTMUX_EXP_PROBE ?? 'missing')"], { env, encoding: "utf8" });
        expect(out.stdout).toBe("arrived");
    });
});
