// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower's pure helpers: formatting, sorting and filtering. The model and view
// hold no logic of their own beyond wiring these to signals.

import type { TowerProcess, TowerTask } from "@/app/store/rpc-api";
import { formatBytes } from "@/util/format-bytes";

/** `machine`: percent of the whole machine, 0–100 whatever the core count
 *  (Windows Task Manager). `core`: percent of one core, can pass 100 (`top`,
 *  Activity Monitor, the pane badge). */
export type CpuMode = "machine" | "core";

export type TowerView = "tasks" | "host";

export type SortKey = "name" | "cpu" | "mem" | "count";
export interface Sort {
    key: SortKey;
    desc: boolean;
}

/** The server sends CPU as a fraction of one core. */
export function cpuPercent(fraction: number, cpuCount: number, mode: CpuMode): number {
    return mode === "core" ? fraction * 100 : (fraction * 100) / Math.max(1, cpuCount);
}

/** "—" when unknown (no rate yet, or the OS wouldn't say). */
export function formatCpu(fraction: number | undefined, cpuCount: number, mode: CpuMode): string {
    if (fraction == null || !Number.isFinite(fraction)) return "—";
    const pct = cpuPercent(fraction, cpuCount, mode);
    if (pct > 0 && pct < 0.1) return "<0.1%";
    return `${pct < 10 ? pct.toFixed(1) : Math.round(pct)}%`;
}

export function formatMem(bytes: number | undefined): string {
    return bytes == null ? "—" : formatBytes(bytes);
}

/** Unknown values sort as the smallest, so "—" rows sink under a descending
 *  sort instead of floating to the top. */
function compareNumbers(a: number | undefined, b: number | undefined): number {
    return (a ?? -1) - (b ?? -1);
}

function compareWith<T>(sort: Sort, value: (x: T) => number | undefined, name: (x: T) => string) {
    return (a: T, b: T): number => {
        const c =
            sort.key === "name"
                ? name(a).localeCompare(name(b), undefined, { sensitivity: "base" })
                : compareNumbers(value(a), value(b));
        // Ties keep a stable, readable order: by name.
        const ordered = c !== 0 ? c : name(a).localeCompare(name(b));
        return sort.desc ? -ordered : ordered;
    };
}

export function sortTasks(tasks: TowerTask[], sort: Sort): TowerTask[] {
    const value = (t: TowerTask) => (sort.key === "cpu" ? t.cpu : sort.key === "mem" ? t.mem : t.processes.length);
    return [...tasks].sort(compareWith(sort, value, (t) => t.label));
}

export function sortProcesses(processes: TowerProcess[], sort: Sort): TowerProcess[] {
    const value = (p: TowerProcess) => (sort.key === "cpu" ? p.cpu : sort.key === "mem" ? p.mem : p.pid);
    return [...processes].sort(compareWith(sort, value, (p) => p.name));
}

/** Every word of `query` appears in the name, the PID or the task's label. */
export function filterProcesses(
    processes: TowerProcess[],
    query: string,
    taskLabel: (taskId: string) => string | undefined
): TowerProcess[] {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (words.length === 0) return processes;
    return processes.filter((p) => {
        const hay = `${p.name} ${p.pid} ${p.task ? (taskLabel(p.task) ?? "") : ""}`.toLowerCase();
        return words.every((w) => hay.includes(w));
    });
}

/** Clicking a column sorts by it; clicking it again flips the direction.
 *  Numbers start descending (biggest first), names ascending. */
export function nextSort(current: Sort, key: SortKey): Sort {
    if (current.key === key) return { key, desc: !current.desc };
    return { key, desc: key !== "name" };
}

const ROLE_LABELS: Record<NonNullable<TowerProcess["role"]>, string> = {
    main: "the pane's own process",
    support: "started for the agent (MCP server, console host)",
    started: "started by the agent or in the terminal",
};

export function roleLabel(role: TowerProcess["role"]): string | undefined {
    return role ? ROLE_LABELS[role] : undefined;
}

const TRACKING_LABELS: Record<string, string> = {
    high: "Every process it starts is tracked (Job Object or cgroup).",
    best_effort: "Found by scanning: a process that detaches itself may be missed.",
    tree: "Found from the pane's process tree: a process that detaches itself is followed until it exits.",
};

export function trackingLabel(tracking: string): string {
    return TRACKING_LABELS[tracking] ?? tracking;
}

/** A process row's hover text: everything the row has no column for. */
export function processDetail(p: TowerProcess, memoryMetric: string): string {
    const lines = [`PID ${p.pid}${p.ppid != null ? ` · parent ${p.ppid}` : ""}`];
    if (p.started_at_ms != null) lines.push(`Started ${new Date(p.started_at_ms).toLocaleString()}`);
    if (p.mem != null) lines.push(`Memory (${memoryMetric}): ${formatBytes(p.mem)}`);
    if (p.mem_resident != null) lines.push(`Resident / working set: ${formatBytes(p.mem_resident)}`);
    if (p.mem_commit != null) lines.push(`Committed: ${formatBytes(p.mem_commit)}`);
    if (p.mem == null && p.cpu == null) lines.push("CPU and memory need administrator rights to read.");
    const role = roleLabel(p.role);
    if (role) lines.push(`Role: ${role}`);
    return lines.join("\n");
}
