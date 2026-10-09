// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TowerProcess, TowerTask } from "@/app/store/rpc-api";
import { describe, expect, it } from "vitest";
import {
    cpuPercent,
    filterProcesses,
    formatCpu,
    formatMem,
    nextSort,
    processDetail,
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
