// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TowerProcess, TowerTask } from "@/app/store/rpc-api";
import { describe, expect, it } from "vitest";
import {
    count,
    cpuPercent,
    filterProcesses,
    formatCpu,
    formatMem,
    groupProcesses,
    nextSort,
    processDetail,
    sortGroups,
    sortProcesses,
    sortTasks,
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
