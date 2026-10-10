// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower's pure helpers: formatting, sorting and filtering. The model and view
// hold no logic of their own beyond wiring these to signals.

import type { TowerProcess, TowerSnapshot, TowerTask } from "@/app/store/rpc-api";
import { formatBytes } from "@/util/format-bytes";

/** `machine`: percent of the whole machine, 0–100 whatever the core count
 *  (Windows Task Manager). `core`: percent of one core, can pass 100 (`top`,
 *  Activity Monitor, the pane badge). */
export type CpuMode = "machine" | "core";

/** `agents`: the rail of agents and what each runs. `processes`: every
 *  process on the machine. SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §3. */
export type TowerView = "agents" | "processes";

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

/** "1 task", "3 tasks". */
export function count(n: number, noun: string): string {
    return `${n} ${noun}${n === 1 ? "" : noun.endsWith("s") ? "es" : "s"}`;
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

/** The processes of one app (one executable name), as Task Manager groups
 *  them: their combined CPU and memory, and the task they belong to when
 *  they all belong to the same one. */
export interface ProcessGroup {
    /** The name, lowercased: what the processes are grouped by. */
    key: string;
    name: string;
    processes: TowerProcess[];
    cpu?: number;
    mem?: number;
    task?: string;
}

/** Group processes by executable name (`chrome.exe` ×24 → one group). */
export function groupProcesses(processes: TowerProcess[]): ProcessGroup[] {
    const byName = new Map<string, TowerProcess[]>();
    for (const p of processes) {
        const key = (p.name || `pid ${p.pid}`).toLowerCase();
        const list = byName.get(key);
        if (list) list.push(p);
        else byName.set(key, [p]);
    }
    const sum = (xs: (number | undefined)[]) => {
        const known = xs.filter((x): x is number => x != null);
        return known.length ? known.reduce((a, b) => a + b, 0) : undefined;
    };
    return [...byName].map(([key, list]) => {
        const tasks = new Set(list.map((p) => p.task));
        return {
            key,
            name: list[0].name || `PID ${list[0].pid}`,
            processes: list,
            cpu: sum(list.map((p) => p.cpu)),
            mem: sum(list.map((p) => p.mem)),
            task: tasks.size === 1 ? list[0].task : undefined,
        };
    });
}

/** Groups sorted like processes; the count column sorts by group size. */
export function sortGroups(groups: ProcessGroup[], sort: Sort): ProcessGroup[] {
    const value = (g: ProcessGroup) => (sort.key === "cpu" ? g.cpu : sort.key === "mem" ? g.mem : g.processes.length);
    return [...groups].sort(compareWith(sort, value, (g) => g.name));
}

/** How the Processes view groups its list: by who started each process (the
 *  agent, a terminal, AgentMux, or nothing AgentMux knows), by app, or not
 *  at all. SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §3.2. */
export type ProcessGrouping = "agent" | "app" | "none";

/** The processes one owner started, for the Processes view grouped by agent. */
export interface OwnerGroup {
    /** The agent's task id, or `terminals`, `agentmux`, `other`. */
    key: string;
    kind: RailKind;
    label: string;
    processes: TowerProcess[];
    cpu?: number;
    mem?: number;
}

/** Who started a process, as the Agents rail and the Processes groups name
 *  it: its agent's task id, `terminals`, `agentmux`, or `other` (no task, or
 *  a task that isn't listed). */
export function ownerOf(p: TowerProcess, tasksById: ReadonlyMap<string, TowerTask>): [string, RailKind, string] {
    const t = p.task ? tasksById.get(p.task) : undefined;
    return !t
        ? [OTHER_ID, "other", "Other processes"]
        : t.kind === "agent"
          ? [t.id, "agent", t.label]
          : t.kind === "terminal"
            ? [TERMINALS_ID, "terminals", "Terminals"]
            : [AGENTMUX_ID, "agentmux", "AgentMux"];
}

/** Every process under its owner (`ownerOf`): one group per agent, then
 *  every terminal's processes together, AgentMux's, and the rest. Groups
 *  without processes are left out. */
export function ownerGroups(processes: TowerProcess[], tasks: TowerTask[]): OwnerGroup[] {
    const byId = new Map(tasks.map((t) => [t.id, t]));
    const groups = new Map<string, OwnerGroup>();
    for (const p of processes) {
        const [key, kind, label] = ownerOf(p, byId);
        let g = groups.get(key);
        if (!g) {
            g = { key, kind, label, processes: [] };
            groups.set(key, g);
        }
        g.processes.push(p);
    }
    return [...groups.values()].map((g) => ({
        ...g,
        cpu: sumKnown(g.processes.map((p) => p.cpu)),
        mem: sumKnown(g.processes.map((p) => p.mem)),
    }));
}

const FIXED_ORDER: Record<RailKind, number> = { agent: 0, terminals: 1, agentmux: 2, other: 3 };

/** Agents in `sort` order (by their totals, or count), then Terminals,
 *  AgentMux and the other processes, always last and in that order. */
export function sortOwnerGroups(groups: OwnerGroup[], sort: Sort): OwnerGroup[] {
    const value = (g: OwnerGroup) => (sort.key === "cpu" ? g.cpu : sort.key === "mem" ? g.mem : g.processes.length);
    const byAgent = compareWith(sort, value, (g: OwnerGroup) => g.label);
    return [...groups].sort(
        (a, b) => FIXED_ORDER[a.kind] - FIXED_ORDER[b.kind] || (a.kind === "agent" ? byAgent(a, b) : 0)
    );
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
        const hay = `${p.name} ${p.pid} ${p.task ? (taskLabel(p.task) ?? "") : ""} ${p.detail ?? ""}`.toLowerCase();
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

// ── The Agents view (SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §3.1) ──

/** What a rail entry stands for: one agent pane, or one of the fixed entries
 *  under the agents that make the rail add up to the machine. */
export type RailKind = "agent" | "terminals" | "agentmux" | "other";

export const TERMINALS_ID = "terminals";
export const OTHER_ID = "other";
/** `TowerTask::id` of AgentMux's own task (tower_sampler.rs). */
export const AGENTMUX_ID = "agentmux";

export interface RailEntry {
    /** The agent's task (its block id), or a fixed entry's id. */
    id: string;
    kind: RailKind;
    label: string;
    /** Fraction of one core; absent when not known yet. */
    cpu?: number;
    mem?: number;
    processes: number;
    /** The tasks the entry stands for (none for "Everything else"). */
    tasks: TowerTask[];
}

function sumKnown(xs: (number | undefined)[]): number | undefined {
    const known = xs.filter((x): x is number => x != null);
    return known.length ? known.reduce((a, b) => a + b, 0) : undefined;
}

/** The rail: each agent pane in the order the sampler lists them (pane
 *  order), then Terminals (every terminal pane), AgentMux, and Everything
 *  else: the machine's totals less every task's, when the snapshot has them. */
export function railEntries(snap: TowerSnapshot): RailEntry[] {
    const entries: RailEntry[] = [];
    const ofKind = (kind: TowerTask["kind"]) => snap.tasks.filter((t) => t.kind === kind);
    for (const t of ofKind("agent")) {
        entries.push({
            id: t.id,
            kind: "agent",
            label: t.label,
            cpu: t.cpu,
            mem: t.mem,
            processes: t.processes.length,
            tasks: [t],
        });
    }
    const terminals = ofKind("terminal");
    if (terminals.length) {
        entries.push({
            id: TERMINALS_ID,
            kind: "terminals",
            label: "Terminals",
            cpu: sumKnown(terminals.map((t) => t.cpu)),
            mem: terminals.reduce((n, t) => n + t.mem, 0),
            processes: terminals.reduce((n, t) => n + t.processes.length, 0),
            tasks: terminals,
        });
    }
    for (const t of ofKind("agentmux")) {
        entries.push({
            id: t.id,
            kind: "agentmux",
            label: t.label,
            cpu: t.cpu,
            mem: t.mem,
            processes: t.processes.length,
            tasks: [t],
        });
    }
    const machine = snap.machine;
    if (machine) {
        const tasksCpu = sumKnown(snap.tasks.map((t) => t.cpu)) ?? 0;
        const tasksMem = snap.tasks.reduce((n, t) => n + t.mem, 0);
        const tasksProcesses = snap.tasks.reduce((n, t) => n + t.processes.length, 0);
        entries.push({
            id: OTHER_ID,
            kind: "other",
            label: "Everything else",
            // A task's CPU can include members that exited since the last
            // sample, which the machine's own total doesn't: never below 0.
            cpu: machine.cpu == null ? undefined : Math.max(0, machine.cpu - tasksCpu),
            mem: Math.max(0, machine.mem - tasksMem),
            processes: Math.max(0, machine.processes - tasksProcesses),
            tasks: [],
        });
    }
    return entries;
}

/** How the rail's agents are ordered; the fixed entries always come last. */
export type RailSort = "pane" | "cpu" | "mem" | "name";

/** The rail ordered by `sort`. A CPU order uses `smoothedCpu` (a recent
 *  average) so rows don't swap on every refresh. */
export function orderRail(
    entries: RailEntry[],
    sort: RailSort,
    smoothedCpu: (id: string) => number | undefined
): RailEntry[] {
    const agents = entries.filter((e) => e.kind === "agent");
    const fixed = entries.filter((e) => e.kind !== "agent");
    const by: Record<RailSort, ((a: RailEntry, b: RailEntry) => number) | undefined> = {
        pane: undefined,
        name: (a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" }),
        cpu: (a, b) => (smoothedCpu(b.id) ?? -1) - (smoothedCpu(a.id) ?? -1),
        mem: (a, b) => (b.mem ?? -1) - (a.mem ?? -1),
    };
    const cmp = by[sort];
    return [...(cmp ? [...agents].sort(cmp) : agents), ...fixed];
}

/** One process in a task's tree, with its descendants and their totals. */
export interface ProcessNode {
    process: TowerProcess;
    children: ProcessNode[];
    /** This process and every descendant: what a parent sorts and shows by. */
    cpu?: number;
    mem?: number;
}

/** A task's processes as trees, a parent above what it started
 *  (`claude → bash → cargo → rustc`). A process's parent is trusted only if
 *  it is in the same list and started no later than it: a reused PID is
 *  never a parent. A process without one is a root. */
export function buildProcessTree(processes: TowerProcess[]): ProcessNode[] {
    const byPid = new Map<number, TowerProcess>();
    for (const p of processes) byPid.set(p.pid, p);
    const nodes = new Map<string, ProcessNode>(processes.map((p) => [p.id, { process: p, children: [] }]));
    const roots: ProcessNode[] = [];
    for (const p of processes) {
        const parent = p.ppid != null && p.ppid !== p.pid ? byPid.get(p.ppid) : undefined;
        const trusted =
            parent != null &&
            (parent.started_at_ms == null || p.started_at_ms == null || parent.started_at_ms <= p.started_at_ms);
        const node = nodes.get(p.id)!;
        if (trusted) nodes.get(parent.id)!.children.push(node);
        else roots.push(node);
    }
    // A cycle (two processes naming each other) would leave both unreached:
    // anything not under a root becomes one.
    const reached = new Set<string>();
    const walk = (n: ProcessNode) => {
        if (reached.has(n.process.id)) return;
        reached.add(n.process.id);
        n.children.forEach(walk);
    };
    roots.forEach(walk);
    for (const n of nodes.values()) {
        if (!reached.has(n.process.id)) {
            n.children = n.children.filter((c) => !reached.has(c.process.id));
            roots.push(n);
            walk(n);
        }
    }
    const total = (n: ProcessNode, seen: Set<string>): void => {
        seen.add(n.process.id);
        n.children = n.children.filter((c) => !seen.has(c.process.id));
        n.children.forEach((c) => total(c, seen));
        n.cpu = sumKnown([n.process.cpu, ...n.children.map((c) => c.cpu)]);
        n.mem = sumKnown([n.process.mem, ...n.children.map((c) => c.mem)]);
    };
    const seen = new Set<string>();
    roots.forEach((r) => total(r, seen));
    return roots;
}

/** One line of the tree: a process (with its own subtree), or several
 *  siblings with the same name and nothing under them (`rustc.exe ×6`). */
export type TreeLine =
    | { kind: "process"; key: string; node: ProcessNode }
    | { kind: "many"; key: string; name: string; nodes: ProcessNode[]; cpu?: number; mem?: number };

/** What a process row is called: what one of AgentMux's own processes is
 *  ("GPU", "Renderer"), else its executable's name. */
export function processLabel(p: TowerProcess): string {
    return p.detail || p.name || `PID ${p.pid}`;
}

/** Siblings in `sort` order (by their subtree totals), with same-named
 *  childless ones folded into one line unless `fold` is false. AgentMux's
 *  CEF processes all run one executable, so they fold by what they are
 *  (`processLabel`), not by that shared name. */
export function treeLines(siblings: ProcessNode[], sort: Sort, opts: { fold?: boolean } = {}): TreeLine[] {
    const fold = opts.fold ?? true;
    const value = (n: ProcessNode) => (sort.key === "cpu" ? n.cpu : sort.key === "mem" ? n.mem : n.process.pid);
    const sorted = [...siblings].sort(compareWith(sort, value, (n) => processLabel(n.process)));
    const leavesByName = new Map<string, ProcessNode[]>();
    for (const n of sorted) {
        if (n.children.length) continue;
        const key = processLabel(n.process).toLowerCase();
        const list = leavesByName.get(key);
        if (list) list.push(n);
        else leavesByName.set(key, [n]);
    }
    const lines: TreeLine[] = [];
    const placed = new Set<string>();
    for (const n of sorted) {
        const key = processLabel(n.process).toLowerCase();
        const group = !fold || n.children.length ? undefined : leavesByName.get(key);
        if (group && group.length > 1) {
            if (placed.has(key)) continue;
            placed.add(key);
            lines.push({
                kind: "many",
                key: `many:${key}`,
                name: processLabel(n.process),
                nodes: group,
                cpu: sumKnown(group.map((g) => g.cpu)),
                mem: sumKnown(group.map((g) => g.mem)),
            });
        } else {
            lines.push({ kind: "process", key: n.process.id, node: n });
        }
    }
    return lines;
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
