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
import { allocatorRoots, dumpedProcesses } from "./lib/sample.mjs";
import { compare, comparisonTable, groupOf, groupSample, inMegabytes, median, MB, steady, table } from "./lib/summary.mjs";

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

    it("refuses to compare runs of a different layout, length or conversation", () => {
        const run = { layout: { panes: 4, tabs: [4] }, rounds: 10, seed: 1 };
        expect(mismatch(run, { ...run })).toBeNull();
        expect(mismatch(run, { ...run, layout: { panes: 12, tabs: [4, 2, 4, 2] } })).toMatch(/different layouts/);
        expect(mismatch(run, { ...run, rounds: 20 })).toMatch(/different rounds/);
        expect(mismatch(run, { ...run, seed: 2 })).toMatch(/different conversations/);
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
