// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TowerProcess, TowerSnapshot, TowerTask } from "@/app/store/rpc-api";
import { describe, expect, it } from "vitest";
import {
    buildProcessTree,
    count,
    cpuPercent,
    filterProcesses,
    formatCpu,
    formatMem,
    groupProcesses,
    nextSort,
    orderRail,
    processDetail,
    railEntries,
    sortGroups,
    sortProcesses,
    sortTasks,
    processLabel,
    treeLines,
} from "./tower-util";

const proc = (pid: number, name: string, cpu?: number, mem?: number, task?: string): TowerProcess => ({
    id: `${pid}:1`,
    pid,
    name,
    cpu,
    mem,
    task,
});

const task = (id: string, label: string, cpu: number | undefined, mem: number, n: number): TowerTask => ({
    id,
    kind: "agent",
    label,
    tracking: "high",
    cpu,
    cpu_account: true,
    mem,
    processes: Array.from({ length: n }, (_, i) => proc(i + 1, "p")),
});

describe("CPU", () => {
    it("is a share of the machine or of one core", () => {
        // 1.5 cores busy on an 8-core machine.
        expect(cpuPercent(1.5, 8, "machine")).toBeCloseTo(18.75);
        expect(cpuPercent(1.5, 8, "core")).toBeCloseTo(150);
        expect(formatCpu(1.5, 8, "machine")).toBe("19%");
        expect(formatCpu(1.5, 8, "core")).toBe("150%");
        expect(formatCpu(0.05, 8, "machine")).toBe("0.6%");
        expect(formatCpu(0.0001, 8, "machine")).toBe("<0.1%");
        expect(formatCpu(0, 8, "machine")).toBe("0.0%");
    });

    it("shows a dash, never zero, when unknown", () => {
        expect(formatCpu(undefined, 8, "machine")).toBe("—");
        expect(formatMem(undefined)).toBe("—");
        expect(formatMem(1536 * 1024 * 1024)).toBe("1.5 GB");
    });
});

describe("counts", () => {
    it("are singular for one", () => {
        expect(count(1, "task")).toBe("1 task");
        expect(count(3, "task")).toBe("3 tasks");
        expect(count(1, "process")).toBe("1 process");
        expect(count(0, "process")).toBe("0 processes");
    });
});

describe("sorting", () => {
    const tasks = [
        task("a", "beta", 0.5, 100, 3),
        task("b", "Alpha", undefined, 300, 1),
        task("c", "gamma", 2, 200, 2),
    ];

    it("sorts numbers biggest first and unknowns last", () => {
        expect(sortTasks(tasks, { key: "cpu", desc: true }).map((t) => t.id)).toEqual(["c", "a", "b"]);
        expect(sortTasks(tasks, { key: "mem", desc: true }).map((t) => t.id)).toEqual(["b", "c", "a"]);
        expect(sortTasks(tasks, { key: "count", desc: true }).map((t) => t.id)).toEqual(["a", "c", "b"]);
    });

    it("sorts names without regard to case", () => {
        expect(sortTasks(tasks, { key: "name", desc: false }).map((t) => t.label)).toEqual(["Alpha", "beta", "gamma"]);
    });

    it("does not reorder its input", () => {
        const before = tasks.map((t) => t.id);
        sortTasks(tasks, { key: "cpu", desc: true });
        expect(tasks.map((t) => t.id)).toEqual(before);
    });

    it("sorts processes by PID for the count column", () => {
        const ps = [proc(30, "x"), proc(4, "y"), proc(12, "z")];
        expect(sortProcesses(ps, { key: "count", desc: false }).map((p) => p.pid)).toEqual([4, 12, 30]);
    });

    it("toggles a column and starts numbers descending", () => {
        expect(nextSort({ key: "cpu", desc: true }, "cpu")).toEqual({ key: "cpu", desc: false });
        expect(nextSort({ key: "cpu", desc: true }, "mem")).toEqual({ key: "mem", desc: true });
        expect(nextSort({ key: "cpu", desc: true }, "name")).toEqual({ key: "name", desc: false });
    });
});

describe("filtering the host list", () => {
    const ps = [proc(100, "node.exe", 0, 0, "t1"), proc(200, "chrome.exe"), proc(4100, "node.exe")];
    const label = (id: string) => (id === "t1" ? "AgentX" : undefined);

    it("matches every word against name, PID and task", () => {
        expect(filterProcesses(ps, "node", label).map((p) => p.pid)).toEqual([100, 4100]);
        expect(filterProcesses(ps, "node agentx", label).map((p) => p.pid)).toEqual([100]);
        expect(filterProcesses(ps, "41", label).map((p) => p.pid)).toEqual([4100]);
        expect(filterProcesses(ps, "  ", label)).toBe(ps);
    });

    it("also matches what one of AgentMux's own processes is", () => {
        const gpu = { ...proc(300, "agentmux-0.59.18.exe"), detail: "GPU" };
        const renderer = { ...proc(301, "agentmux-0.59.18.exe"), detail: "Renderer" };
        expect(filterProcesses([gpu, renderer], "gpu", label).map((p) => p.pid)).toEqual([300]);
    });
});

describe("a process's details", () => {
    it("says why an unmeasured process has no numbers", () => {
        expect(processDetail(proc(1, "launchd"), "memory footprint")).toContain("administrator rights");
        const measured = { ...proc(9, "node", 0.1, 2048), mem_resident: 4096, role: "support" as const };
        const text = processDetail(measured, "private working set");
        expect(text).toContain("Memory (private working set): 2 KB");
        expect(text).toContain("Resident / working set: 4 KB");
        expect(text).toContain("MCP server");
        expect(text).not.toContain("administrator");
    });
});

describe("grouping by app", () => {
    const ps = [
        proc(1, "chrome.exe", 0.5, 100, "t1"),
        proc(2, "Chrome.exe", 0.25, 50, "t1"),
        proc(3, "node.exe", undefined, 10),
        proc(4, "node.exe", 0.1, undefined, "t2"),
        proc(5, "System", 0.01, 1),
    ];

    it("groups by name, sums what is known, and names a shared task", () => {
        const g = groupProcesses(ps);
        const chrome = g.find((x) => x.key === "chrome.exe")!;
        expect(chrome.processes.map((p) => p.pid)).toEqual([1, 2]);
        expect(chrome.cpu).toBeCloseTo(0.75);
        expect(chrome.mem).toBe(150);
        expect(chrome.task).toBe("t1");
        const node = g.find((x) => x.key === "node.exe")!;
        expect(node.cpu).toBeCloseTo(0.1);
        expect(node.mem).toBe(10);
        expect(node.task).toBeUndefined();
    });

    it("sorts groups by their totals or their size", () => {
        const g = groupProcesses(ps);
        expect(sortGroups(g, { key: "cpu", desc: true }).map((x) => x.key)).toEqual([
            "chrome.exe",
            "node.exe",
            "system",
        ]);
        expect(sortGroups(g, { key: "count", desc: true })[2].key).toBe("system");
    });
});

describe("the Agents view's rail", () => {
    const t = (id: string, kind: TowerTask["kind"], cpu: number | undefined, mem: number, n = 1): TowerTask => ({
        id,
        kind,
        label: id,
        tracking: "high",
        cpu,
        cpu_account: true,
        mem,
        processes: Array.from({ length: n }, (_, i) => proc(i + 1, `${id}-${i}`)),
    });
    const snap = (tasks: TowerTask[], machine?: TowerSnapshot["machine"]): TowerSnapshot => ({
        ts_ms: 1,
        hostname: "h",
        os: "linux",
        cpu_count: 4,
        memory_metric: "m",
        interval_ms: 2000,
        remote: false,
        tasks,
        machine,
    });

    it("lists agents in pane order, then Terminals, AgentMux and everything else", () => {
        const entries = railEntries(
            snap(
                [
                    t("b", "agent", 1, 10),
                    t("term1", "terminal", 0.5, 5, 2),
                    t("a", "agent", 2, 20),
                    t("term2", "terminal", undefined, 5),
                    t("agentmux", "agentmux", 0.25, 7),
                ],
                { cpu: 4, mem: 100, processes: 50 }
            )
        );
        expect(entries.map((e) => [e.id, e.kind])).toEqual([
            ["b", "agent"],
            ["a", "agent"],
            ["terminals", "terminals"],
            ["agentmux", "agentmux"],
            ["other", "other"],
        ]);
        const terminals = entries[2];
        // An unknown CPU isn't counted as 0; it just isn't added.
        expect([terminals.cpu, terminals.mem, terminals.processes, terminals.tasks.length]).toEqual([0.5, 10, 3, 2]);
        const other = entries[4];
        expect([other.cpu, other.mem, other.processes]).toEqual([0.25, 53, 44]);
    });

    it("everything else is never below zero, and unknown without the machine's CPU", () => {
        const [, other] = railEntries(snap([t("a", "agent", 3, 90)], { cpu: 2, mem: 50, processes: 0 }));
        expect([other.cpu, other.mem, other.processes]).toEqual([0, 0, 0]);
        expect(railEntries(snap([t("a", "agent", 1, 1)], { mem: 5, processes: 3 }))[1].cpu).toBeUndefined();
        expect(railEntries(snap([t("a", "agent", 1, 1)])).map((e) => e.id)).toEqual(["a"]);
    });

    it("orders agents by name, memory or smoothed CPU, the fixed entries always last", () => {
        const entries = railEntries(
            snap([t("b", "agent", 1, 30), t("a", "agent", 2, 10), t("agentmux", "agentmux", 9, 99)], {
                cpu: 20,
                mem: 200,
                processes: 9,
            })
        );
        const ids = (sort: Parameters<typeof orderRail>[1], smooth: Record<string, number> = {}) =>
            orderRail(entries, sort, (id) => smooth[id]).map((e) => e.id);
        expect(ids("pane")).toEqual(["b", "a", "agentmux", "other"]);
        expect(ids("name")).toEqual(["a", "b", "agentmux", "other"]);
        expect(ids("mem")).toEqual(["b", "a", "agentmux", "other"]);
        // The recent average decides, not this sample: b was busier.
        expect(ids("cpu", { a: 0.5, b: 1.5 })).toEqual(["b", "a", "agentmux", "other"]);
    });
});

describe("process trees", () => {
    const p = (pid: number, ppid: number | undefined, name: string, cpu?: number, started?: number): TowerProcess => ({
        id: `${pid}:${started ?? 0}`,
        pid,
        ppid,
        name,
        cpu,
        mem: cpu == null ? undefined : 1,
        started_at_ms: started,
    });

    it("puts each process under its parent, with the subtree's totals", () => {
        const [root] = buildProcessTree([
            p(1, 0, "claude", 0.5, 10),
            p(2, 1, "bash", 0, 20),
            p(3, 2, "cargo", 0.25, 30),
            p(4, 3, "rustc", 1, 40),
        ]);
        expect(root.process.name).toBe("claude");
        expect(root.children[0].children[0].children[0].process.name).toBe("rustc");
        expect(root.cpu).toBe(1.75);
        expect(root.mem).toBe(4);
        expect(root.children[0].children[0].cpu).toBe(1.25);
    });

    it("never trusts a parent that started after its child: a reused PID", () => {
        const roots = buildProcessTree([p(1, 0, "old-parent-gone", 0, 500), p(2, 1, "child", 0, 100)]);
        expect(roots.map((r) => r.process.name).sort()).toEqual(["child", "old-parent-gone"]);
        expect(roots.every((r) => r.children.length === 0)).toBe(true);
    });

    it("survives two processes naming each other as parent", () => {
        const roots = buildProcessTree([p(1, 2, "a"), p(2, 1, "b")]);
        const names: string[] = [];
        const walk = (n: (typeof roots)[number]) => {
            names.push(n.process.name);
            n.children.forEach(walk);
        };
        roots.forEach(walk);
        expect(names.sort()).toEqual(["a", "b"]);
    });

    it("folds AgentMux's CEF processes by what they are, not by their shared executable", () => {
        const exe = "agentmux-0.59.18.exe";
        const withDetail = (pid: number, cpu: number, detail: string) => ({ ...p(pid, 1, exe, cpu, pid), detail });
        const [root] = buildProcessTree([
            { ...p(1, 0, exe, 0, 1), detail: "Main process" },
            withDetail(2, 0.3, "GPU"),
            withDetail(3, 0.2, "Renderer"),
            withDetail(4, 0.1, "Renderer"),
            withDetail(5, 0.05, "Network service"),
        ]);
        const lines = treeLines(root.children, { key: "cpu", desc: true });
        expect(lines.map((l) => (l.kind === "many" ? `${l.name} x${l.nodes.length}` : processLabel(l.node.process)))).toEqual([
            "GPU",
            "Renderer x2",
            "Network service",
        ]);
    });

    it("folds same-named childless siblings, sorted by the subtree", () => {
        const [root] = buildProcessTree([
            p(1, 0, "cargo", 0, 1),
            p(2, 1, "rustc", 1, 2),
            p(3, 1, "rustc", 0.5, 3),
            p(4, 1, "build-script", 2, 4),
            p(5, 4, "cc", 0.1, 5),
        ]);
        const lines = treeLines(root.children, { key: "cpu", desc: true });
        expect(lines.map((l) => (l.kind === "many" ? `${l.name} x${l.nodes.length}` : l.node.process.name))).toEqual([
            "build-script",
            "rustc x2",
        ]);
        const many = lines[1];
        expect(many.kind === "many" && many.cpu).toBe(1.5);
        // Opened, the same siblings are listed one by one.
        const unfolded = treeLines(many.kind === "many" ? many.nodes : [], { key: "cpu", desc: true }, { fold: false });
        expect(unfolded.map((l) => l.kind)).toEqual(["process", "process"]);
    });
});
