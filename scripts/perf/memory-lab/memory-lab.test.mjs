// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the memory lab's grouping, steady values, comparison and
// conversation (scripts/perf/memory-lab). Starting and driving an instance
// needs a build and isn't tested here.

import { describe, expect, it } from "vitest";

import { mismatch } from "./compare.mjs";
import { generateConversation } from "./lib/conversation.mjs";
import { portableExe } from "./lib/driver.mjs";
import { draftText, recycleMark, recycleSaving, scrollArmed, scrollRestored } from "./lib/recycle.mjs";
import { allocatorRoots, dumpedProcesses, lastSample, mainPage, withChromiumRoles } from "./lib/sample.mjs";
import { agentsWithoutTurns, compare, comparisonTable, groupOf, groupSample, inMegabytes, median, MB, steady, table, transcriptGrowthProblem } from "./lib/summary.mjs";

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
