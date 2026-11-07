// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Processes view's table (`HostTable` in tower-view.tsx), checked path by
// path: the summary line and its load colours, the "only" and kind filters,
// System folded into one row, the order each grouping is sorted in, the
// 400-row limit, and the first list's busiest agents and kinds opening by
// themselves. The rest of Tower is in tower-view.test.tsx.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const sample = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        TowerSampleCommand: (...args: unknown[]) => sample(...args),
        TowerPeersCommand: () => Promise.resolve({ peers: [] }),
        TowerPairCommand: () => Promise.resolve({}),
        TowerForgetCommand: () => Promise.resolve({}),
        SetConfigCommand: () => Promise.resolve({}),
    },
}));
vi.mock("@/store/global", () => ({ settingsAtom: () => ({}) }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => undefined }));
vi.mock("@/app/store/remotes-store", () => ({ remotesList: () => () => [] }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (t: string, id: string) => `${t}:${id}`,
    getObjectValue: () => ({ meta: { "frame:activebordercolor": "#ff8800" } }),
}));
vi.mock("@/app/block/pane-identity", () => ({
    blockRoleColor: (meta: Record<string, unknown> | undefined) => meta?.["frame:activebordercolor"],
    isLightThemeActive: () => false,
}));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { TowerProcess, TowerSnapshot } from "@/app/store/rpc-api";
import { TowerViewModel } from "./tower-model";
import { TowerView } from "./tower-view";

const GB = 1024 * 1024 * 1024;

type Task = TowerSnapshot["tasks"][number];

const proc = (pid: number, name: string, extra: Partial<TowerProcess> = {}): TowerProcess => ({
    id: `${pid}:1`,
    pid,
    name,
    ...extra,
});

const agentTask = (id: string, label: string): Task => ({
    id,
    kind: "agent",
    label,
    tracking: "high",
    cpu: 0,
    cpu_account: true,
    mem: 1,
    processes: [],
});

const terminalTask = (id: string, label: string): Task => ({ ...agentTask(id, label), kind: "terminal" });

interface HostOpts {
    processes: TowerProcess[];
    tasks?: Task[];
    total?: number;
    matched?: number;
    unmeasured?: number;
    cpu?: number;
    mem?: number;
    memTotal?: number;
}

function hostSnap(o: HostOpts): TowerSnapshot {
    return {
        ts_ms: 1,
        hostname: "narko",
        os: "windows",
        cpu_count: 4,
        memory_metric: "private working set",
        interval_ms: 2000,
        remote: false,
        tasks: o.tasks ?? [],
        machine: { cpu: 1, mem: GB, processes: o.processes.length, mem_total: o.memTotal },
        host: {
            processes: o.processes,
            unmeasured: o.unmeasured ?? 0,
            cpu: o.cpu ?? 0.5,
            mem: o.mem ?? GB,
            total: o.total ?? o.processes.length,
            matched: o.matched ?? o.total ?? o.processes.length,
        },
    };
}

const [meta, setMeta] = createSignal<Record<string, unknown>>({});
const setMetaMock = vi.fn((patch: Record<string, unknown>) => {
    setMeta((prev) => {
        const next = { ...prev, ...patch };
        for (const [k, v] of Object.entries(patch)) if (v === null) delete next[k];
        return next;
    });
    return Promise.resolve();
});

function renderTower() {
    return render(() => {
        const model = new TowerViewModel({
            blockId: "tower-1",
            meta: meta as unknown as PaneTabHostContext["meta"],
            setMeta: setMetaMock,
            isFocused: () => true,
            visibility: () => "active",
        });
        return <TowerView model={model} />;
    });
}

/** Shows the Processes view of this snapshot, grouped as the pane's meta says. */
async function show(snap: TowerSnapshot, paneMeta: Record<string, unknown> = {}) {
    sample.mockImplementation(() => Promise.resolve(snap));
    setMeta({ "tower:view": "processes", ...paneMeta });
    renderTower();
    await screen.findByRole("table", { name: "Processes" });
}

const table = () => screen.getByRole("table", { name: "Processes" });
const bodyRows = () => Array.from(table().querySelectorAll("tbody tr"));
const rowTexts = () =>
    bodyRows().map((r) => (r.querySelector(".tower-name")?.textContent ?? "").replace(/\s+/g, " ").trim());
const rowIndex = (name: string) => rowTexts().findIndex((t) => t.includes(name));
const summary = () => document.querySelector(".tower-summary") as HTMLElement;
const summaryText = () => (summary().textContent ?? "").replace(/\s+/g, " ").trim();
const header = (name: string) => within(table()).getByRole("columnheader", { name });

describe("Tower's Processes table", () => {
    beforeEach(() => {
        setMeta({});
    });
    afterEach(() => {
        cleanup();
        vi.clearAllMocks();
        vi.useRealTimers();
        setMeta({});
    });

    describe("the summary line", () => {
        it("counts every process on the machine, then memory and CPU, each in its own total", async () => {
            // 3 listed of 3 matching, 10 on the machine: the count is the machine's.
            const snap = hostSnap({
                processes: [proc(1, "a.exe"), proc(2, "b.exe"), proc(3, "c.exe")],
                total: 10,
                matched: 3,
                mem: 3 * GB,
                cpu: 2,
            });
            await show(snap, { "tower:group": "none" });
            expect(summaryText()).toBe("10 processes · Memory 3.0 GB · CPU 50%");
            expect(summary().querySelector(".tower-total--mem")).toHaveTextContent("3.0 GB");
            expect(summary().querySelector(".tower-total--cpu")).toHaveTextContent("50%");
        });

        it("says 1 process for one", async () => {
            await show(hostSnap({ processes: [proc(1, "a.exe")] }), { "tower:group": "none" });
            expect(summaryText()).toMatch(/^1 process · Memory/);
        });

        it("colours the CPU total by its share of the machine, the memory total by its own thresholds", async () => {
            // 3.6 of 4 cores is 90% (high); 7.5 of 10 GB is 75% (warn for memory).
            await show(hostSnap({ processes: [proc(1, "a.exe")], cpu: 3.6, mem: 7.5 * GB, memTotal: 10 * GB }));
            expect(summary().querySelector(".tower-total--cpu")).toHaveClass("tower-level--high");
            expect(summary().querySelector(".tower-total--mem")).toHaveClass("tower-level--warn");
            expect(summary().querySelector(".tower-total--mem")).not.toHaveClass("tower-level--high");
            expect(summary().querySelector(".tower-total--cpu")).not.toHaveClass("tower-level--warn");
        });

        it("a share that is warning for CPU but not for memory colours only the CPU", async () => {
            // 65% of the cores and 65% of the memory: CPU warns from 60%, memory from 70%.
            await show(hostSnap({ processes: [proc(1, "a.exe")], cpu: 2.6, mem: 6.5 * GB, memTotal: 10 * GB }));
            expect(summary().querySelector(".tower-total--cpu")).toHaveClass("tower-level--warn");
            expect(summary().querySelector(".tower-total--mem")!.className).not.toMatch(/tower-level/);
        });

        it("colours neither total when both are light", async () => {
            await show(hostSnap({ processes: [proc(1, "a.exe")], cpu: 0.4, mem: GB, memTotal: 10 * GB }));
            expect(summary().querySelector(".tower-total--cpu")!.className).not.toMatch(/tower-level/);
            expect(summary().querySelector(".tower-total--mem")!.className).not.toMatch(/tower-level/);
        });

        it("says how many of the matching it lists when another machine sent only the busiest", async () => {
            const snap = hostSnap({
                processes: [proc(1, "a.exe"), proc(2, "b.exe"), proc(3, "c.exe")],
                total: 10,
                matched: 6,
            });
            await show(snap, { "tower:group": "none" });
            expect(summaryText()).toContain("· showing the 3 busiest and largest of 6 matching");
            const note = Array.from(summary().querySelectorAll(".tower-muted")).find((e) =>
                e.textContent?.includes("showing")
            );
            expect(note).toBeTruthy();
        });

        it("leaves out 'of n matching' when nothing is filtered out", async () => {
            const snap = hostSnap({
                processes: [proc(1, "a.exe"), proc(2, "b.exe"), proc(3, "c.exe")],
                total: 10,
                matched: 10,
            });
            await show(snap, { "tower:group": "none" });
            expect(summaryText()).toContain("· showing the 3 busiest and largest");
            expect(summaryText()).not.toContain("matching");
        });

        it("shows no 'showing the' note when every matching process is listed", async () => {
            const snap = hostSnap({
                processes: [proc(1, "a.exe"), proc(2, "b.exe"), proc(3, "c.exe")],
                total: 10,
                matched: 3,
            });
            await show(snap, { "tower:group": "none" });
            expect(summaryText()).not.toContain("showing the");
        });

        it("counts the processes it can't measure, only when there are some", async () => {
            await show(hostSnap({ processes: [proc(1, "a.exe")], total: 9, unmeasured: 2 }));
            expect(summaryText()).toContain("· 2 owned by other users can't be measured without administrator rights");
            const note = Array.from(summary().querySelectorAll(".tower-muted")).find((e) =>
                e.textContent?.includes("administrator")
            );
            expect(note).toBeTruthy();
            cleanup();
            await show(hostSnap({ processes: [proc(1, "a.exe")], total: 9, unmeasured: 0 }));
            expect(summaryText()).not.toContain("administrator");
        });
    });

    describe("the bar above the table", () => {
        it("has a filter box that narrows the list and clears again", async () => {
            const snap = hostSnap({ processes: [proc(1, "alpha.exe"), proc(2, "bravo.exe")] });
            await show(snap, { "tower:group": "none" });
            const input = screen.getByRole("textbox", { name: "Filter by name, PID or task" });
            expect(input).toHaveAttribute("placeholder", "Filter by name, PID or task");
            expect(input.closest(".ui-filter")).toHaveClass("tower-filter");
            expect(screen.queryByTestId("tower-filter-clear")).toBeNull();
            fireEvent.input(input, { target: { value: "alpha" } });
            await waitFor(() => expect(screen.queryByText("bravo.exe")).toBeNull());
            expect(screen.getByText("alpha.exe")).toBeInTheDocument();
            fireEvent.click(screen.getByTestId("tower-filter-clear"));
            expect(await screen.findByText("bravo.exe")).toBeInTheDocument();
        });

        it("groups by agent, app, type or nothing, and shows the one in use", async () => {
            await show(hostSnap({ processes: [proc(1, "alpha.exe")] }), { "tower:group": "app" });
            const group = screen.getByRole("radiogroup", { name: "Group by" });
            const radio = (name: string) => within(group).getByRole("radio", { name });
            expect(radio("App")).toHaveAttribute("aria-checked", "true");
            expect(radio("Agent")).toHaveAttribute("aria-checked", "false");
            expect(radio("Type")).toHaveAttribute("aria-checked", "false");
            expect(radio("None")).toHaveAttribute("aria-checked", "false");
            expect(radio("Agent")).toHaveAttribute("title", "Under the agent or terminal that started each");
            expect(radio("App")).toHaveAttribute("title", "Processes of the same app together, as Task Manager does");
            expect(radio("Type")).toHaveAttribute("title", "By kind of process: agents, builds, browsers, the system…");
            expect(radio("None")).toHaveAttribute("title", "One flat list");
            fireEvent.click(radio("Type"));
            expect(setMetaMock).toHaveBeenCalledWith({ "tower:group": "type" });
        });

        it("shows None as the one in use for a flat list", async () => {
            await show(hostSnap({ processes: [proc(1, "alpha.exe")] }), { "tower:group": "none" });
            const group = screen.getByRole("radiogroup", { name: "Group by" });
            expect(within(group).getByRole("radio", { name: "None" })).toHaveAttribute("aria-checked", "true");
            expect(within(group).getByRole("radio", { name: "App" })).toHaveAttribute("aria-checked", "false");
        });

        it("has one bar for the summary and one for the filters", async () => {
            await show(hostSnap({ processes: [proc(1, "alpha.exe")] }));
            const bar = document.querySelector(".tower-host-bar") as HTMLElement;
            expect(bar).toBeTruthy();
            expect(within(bar).getByRole("textbox")).toBeInTheDocument();
            expect(summary()).toBeTruthy();
        });
    });

    describe("the column headers", () => {
        it("sort by memory first, and each header sorts by its own column", async () => {
            const snap = hostSnap({ processes: [proc(1, "alpha.exe", { mem: GB, cpu: 0.1 })] });
            await show(snap, { "tower:group": "app" });
            expect(header("Process")).toHaveAttribute("aria-sort", "none");
            expect(header("Memory")).toHaveAttribute("aria-sort", "descending");
            expect(header("CPU")).toHaveAttribute("aria-sort", "none");
            expect(header("Count")).toHaveAttribute("aria-sort", "none");
            fireEvent.click(within(header("CPU")).getByRole("button"));
            expect(header("CPU")).toHaveAttribute("aria-sort", "descending");
            expect(header("Memory")).toHaveAttribute("aria-sort", "none");
            fireEvent.click(within(header("Process")).getByRole("button"));
            expect(header("Process")).toHaveAttribute("aria-sort", "ascending");
            expect(header("CPU")).toHaveAttribute("aria-sort", "none");
            fireEvent.click(within(header("Count")).getByRole("button"));
            expect(header("Count")).toHaveAttribute("aria-sort", "descending");
            expect(header("Process")).toHaveAttribute("aria-sort", "none");
        });

        it("right-aligns the numbers, not the names", async () => {
            await show(hostSnap({ processes: [proc(1, "alpha.exe")] }), { "tower:group": "app" });
            expect(header("Process")).not.toHaveClass("tower-num");
            expect(header("Memory")).toHaveClass("tower-num");
            expect(header("CPU")).toHaveClass("tower-num");
            expect(header("Count")).toHaveClass("tower-num");
        });

        it("calls the last column Count with a hint when grouped, and PID without one in a flat list", async () => {
            await show(hostSnap({ processes: [proc(1, "alpha.exe")] }), { "tower:group": "app" });
            expect(within(header("Count")).getByRole("button")).toHaveAttribute(
                "title",
                "Processes in the group, or the PID of one"
            );
            cleanup();
            await show(hostSnap({ processes: [proc(1, "alpha.exe")] }), { "tower:group": "none" });
            expect(header("PID")).toHaveClass("tower-num");
            expect(within(header("PID")).getByRole("button")).not.toHaveAttribute("title");
            expect(within(table()).queryByRole("columnheader", { name: "Count" })).toBeNull();
        });
    });

    describe("which processes are listed", () => {
        const tasks = [agentTask("block-a", "AgentX"), agentTask("quiet", "Quiet"), terminalTask("term-1", "pwsh")];
        const processes = [
            proc(10, "claude.exe", { task: "block-a", kind: "agents", mem: 4 * GB }),
            proc(11, "cargo.exe", { task: "block-a", kind: "builds", mem: 3 * GB }),
            proc(12, "pwsh.exe", { task: "term-1", kind: "shells", mem: 2 * GB }),
            proc(13, "rustc.exe", { kind: "builds", mem: GB }),
            proc(14, "notepad.exe", { mem: GB / 2 }),
        ];
        const snap = () => hostSnap({ processes, tasks });
        const only = () => screen.getByTestId("tower-only");

        it("narrows to one agent's processes and names it", async () => {
            await show(snap(), { "tower:group": "none", "tower:only": "block-a" });
            expect(only()).toHaveTextContent("Only AgentX");
            expect(rowTexts().join("|")).toMatch(/claude\.exe.*cargo\.exe/);
            expect(screen.queryByText("pwsh.exe")).toBeNull();
            expect(screen.queryByText("rustc.exe")).toBeNull();
            fireEvent.click(within(only()).getByRole("button", { name: "Show all" }));
            expect(setMetaMock).toHaveBeenCalledWith({ "tower:only": null });
            expect(await screen.findByText("rustc.exe")).toBeInTheDocument();
            expect(screen.queryByTestId("tower-only")).toBeNull();
        });

        it("names a terminal's processes Terminals", async () => {
            await show(snap(), { "tower:group": "none", "tower:only": "terminals" });
            expect(only()).toHaveTextContent("Only Terminals");
            expect(screen.getByText("pwsh.exe")).toBeInTheDocument();
            expect(screen.queryByText("claude.exe")).toBeNull();
        });

        it("names the processes no task started Other processes", async () => {
            await show(snap(), { "tower:group": "none", "tower:only": "other" });
            expect(only()).toHaveTextContent("Only Other processes");
            expect(screen.getByText("notepad.exe")).toBeInTheDocument();
            expect(screen.queryByText("claude.exe")).toBeNull();
        });

        it("names an agent that has no process on the list by its task's label", async () => {
            await show(snap(), { "tower:group": "none", "tower:only": "quiet" });
            expect(only()).toHaveTextContent("Only Quiet");
            expect(bodyRows()).toHaveLength(0);
        });

        it("names the agent whose id is the word 'agent' by that agent's label", async () => {
            const odd = hostSnap({
                tasks: [agentTask("block-a", "AgentX"), agentTask("agent", "Zed")],
                processes: [proc(10, "claude.exe", { task: "block-a" }), proc(11, "zed.exe", { task: "agent" })],
            });
            await show(odd, { "tower:group": "none", "tower:only": "agent" });
            expect(only()).toHaveTextContent("Only Zed");
            expect(screen.getByText("zed.exe")).toBeInTheDocument();
            expect(screen.queryByText("claude.exe")).toBeNull();
        });

        it("calls an owner it doesn't know that owner", async () => {
            await show(snap(), { "tower:group": "none", "tower:only": "ghost" });
            expect(only()).toHaveTextContent("Only that owner");
        });

        it("shows no 'Only' bar for either filter when none is set", async () => {
            await show(snap(), { "tower:group": "none" });
            expect(screen.queryByTestId("tower-only")).toBeNull();
            expect(screen.queryByTestId("tower-kind-only")).toBeNull();
            expect(screen.getByText("claude.exe")).toBeInTheDocument();
            expect(screen.getByText("notepad.exe")).toBeInTheDocument();
        });

        it("narrows to one kind of process, with its icon, and Show all drops only that", async () => {
            await show(snap(), { "tower:group": "none", "tower:kind": "builds" });
            const bar = screen.getByTestId("tower-kind-only");
            expect(bar).toHaveTextContent("Only Builds & tools");
            expect(within(bar).getByRole("img", { name: "Builds & tools" })).toBeInTheDocument();
            expect(screen.getByText("cargo.exe")).toBeInTheDocument();
            expect(screen.getByText("rustc.exe")).toBeInTheDocument();
            expect(screen.queryByText("claude.exe")).toBeNull();
            expect(screen.queryByTestId("tower-only")).toBeNull();
            fireEvent.click(within(bar).getByRole("button", { name: "Show all" }));
            expect(setMetaMock).toHaveBeenCalledWith({ "tower:kind": null });
            expect(await screen.findByText("claude.exe")).toBeInTheDocument();
        });

        it("applies an agent and a kind together: both must match", async () => {
            await show(snap(), { "tower:group": "none", "tower:only": "block-a", "tower:kind": "builds" });
            expect(screen.getByText("cargo.exe")).toBeInTheDocument();
            expect(screen.queryByText("claude.exe")).toBeNull();
            expect(screen.queryByText("rustc.exe")).toBeNull();
            expect(screen.getByTestId("tower-only")).toBeInTheDocument();
            expect(screen.getByTestId("tower-kind-only")).toBeInTheDocument();
        });

        it("counts a process the machine sent no kind for as an app", async () => {
            await show(snap(), { "tower:group": "none", "tower:kind": "apps" });
            expect(screen.getByText("notepad.exe")).toBeInTheDocument();
            expect(screen.queryByText("claude.exe")).toBeNull();
        });
    });

    describe("System folded into one row", () => {
        const processes = [
            proc(1, "app.exe", { kind: "apps", mem: 3 * GB }),
            proc(2, "svchost.exe", { kind: "system", mem: GB }),
            proc(3, "svchost.exe", { kind: "system", mem: GB }),
            proc(4, "lsass.exe", { kind: "system", mem: GB / 2 }),
        ];
        const snap = () => hostSnap({ processes });

        it("folds the system's processes into a System row in the list by app", async () => {
            await show(snap(), { "tower:group": "app" });
            expect(screen.getByTestId("tower-kind-system")).toBeInTheDocument();
            expect(screen.getByText("app.exe")).toBeInTheDocument();
            expect(screen.queryByText("lsass.exe")).toBeNull();
            expect(screen.queryByTestId("tower-app-svchost.exe#system")).toBeNull();
            // Opened, it lists them by app: the two svchost.exe as one group.
            fireEvent.click(screen.getByRole("button", { name: "Show System" }));
            expect(screen.getByTestId("tower-app-svchost.exe#system")).toBeInTheDocument();
            expect(screen.getByText("lsass.exe")).toBeInTheDocument();
        });

        it("folds them in the flat list too, and opens to the processes one by one", async () => {
            await show(snap(), { "tower:group": "none" });
            expect(screen.getByTestId("tower-kind-system")).toBeInTheDocument();
            expect(screen.queryByText("lsass.exe")).toBeNull();
            fireEvent.click(screen.getByRole("button", { name: "Show System" }));
            expect(screen.getAllByText("svchost.exe")).toHaveLength(2);
            expect(screen.getByText("lsass.exe")).toBeInTheDocument();
            expect(screen.queryByTestId("tower-app-svchost.exe#system")).toBeNull();
        });

        it("keeps the System row after the other processes, with the system's totals", async () => {
            await show(snap(), { "tower:group": "none" });
            const rows = bodyRows();
            expect(rows[rows.length - 1]).toBe(screen.getByTestId("tower-kind-system"));
            const system = screen.getByTestId("tower-kind-system");
            expect(within(system).getByText("(3)")).toBeInTheDocument();
            expect(within(system).getByText("2.5 GB")).toBeInTheDocument();
        });

        it("doesn't fold in the list by type: System is one of the kinds there", async () => {
            await show(snap(), { "tower:group": "type" });
            expect(screen.getAllByTestId("tower-kind-system")).toHaveLength(1);
            expect(screen.getByTestId("tower-kind-apps")).toBeInTheDocument();
            expect(screen.queryByText("app.exe")).toBeNull();
        });

        it("doesn't fold in the list by agent: the system's processes are under Other", async () => {
            await show(snap(), { "tower:group": "agent" });
            expect(screen.queryByTestId("tower-kind-system")).toBeNull();
            expect(screen.getByTestId("tower-owner-other")).toHaveTextContent("(4)");
        });

        it("lists them where they are when the system is the kind asked for", async () => {
            await show(snap(), { "tower:group": "app", "tower:kind": "system" });
            expect(screen.queryByTestId("tower-kind-system")).toBeNull();
            expect(screen.getByTestId("tower-app-svchost.exe#system")).toBeInTheDocument();
            expect(screen.getByText("lsass.exe")).toBeInTheDocument();
        });

        it("has nothing to fold when the machine sent no kinds", async () => {
            const bare = hostSnap({ processes: [proc(1, "app.exe"), proc(2, "svchost.exe")] });
            await show(bare, { "tower:group": "app" });
            expect(screen.queryByTestId("tower-kind-system")).toBeNull();
            expect(screen.getByText("svchost.exe")).toBeInTheDocument();
        });
    });

    describe("the order of each grouping", () => {
        // Inserted in the order the default sort must undo.
        const processes = [
            proc(30, "gamma.exe", { kind: "shells", mem: GB, cpu: 0.3 }),
            proc(20, "bravo.exe", { kind: "builds", mem: 2 * GB, cpu: 0.5 }),
            proc(10, "alpha.exe", { kind: "browsers", mem: 3 * GB, cpu: 0.1 }),
            proc(40, "dup.exe", { kind: "shells", mem: 1024, cpu: 0 }),
            proc(41, "dup.exe", { kind: "shells", mem: 1024, cpu: 0 }),
        ];
        const snap = () => hostSnap({ processes });
        const click = (name: string) => fireEvent.click(within(header(name)).getByRole("button"));

        it("lists apps biggest first, and by CPU, name or count when a header is clicked", async () => {
            await show(snap(), { "tower:group": "app" });
            const before = (a: string, b: string) => expect(rowIndex(a)).toBeLessThan(rowIndex(b));
            before("alpha.exe", "bravo.exe");
            before("bravo.exe", "gamma.exe");
            click("CPU");
            before("bravo.exe", "gamma.exe");
            before("gamma.exe", "alpha.exe");
            click("Process");
            before("alpha.exe", "bravo.exe");
            before("bravo.exe", "dup.exe");
            click("Count");
            before("dup.exe", "alpha.exe");
        });

        it("lists an app of several processes by the app's total, not by its biggest process", async () => {
            const grouped = [
                proc(1, "alpha.exe", { kind: "apps", mem: 3 * GB, cpu: 0.1 }),
                proc(2, "multi.exe", { kind: "apps", mem: 2 * GB, cpu: 0.1 }),
                proc(3, "multi.exe", { kind: "apps", mem: 2 * GB, cpu: 0.1 }),
                proc(4, "multi.exe", { kind: "apps", mem: 2 * GB, cpu: 0.1 }),
                proc(5, "small.exe", { kind: "apps", mem: GB, cpu: 0.1 }),
            ];
            await show(hostSnap({ processes: grouped }), { "tower:group": "app" });
            // Six GB together, ahead of the one 3 GB process, though none of its own is that big.
            expect(rowIndex("multi.exe")).toBeLessThan(rowIndex("alpha.exe"));
            expect(rowIndex("alpha.exe")).toBeLessThan(rowIndex("small.exe"));
        });

        it("lists the flat list by the same columns, with PID last", async () => {
            await show(snap(), { "tower:group": "none" });
            const before = (a: string, b: string) => expect(rowIndex(a)).toBeLessThan(rowIndex(b));
            before("alpha.exe", "bravo.exe");
            before("bravo.exe", "gamma.exe");
            click("PID");
            before("gamma.exe", "bravo.exe");
            before("bravo.exe", "alpha.exe");
        });

        it("lists the kinds in the list by type biggest first", async () => {
            await show(snap(), { "tower:group": "type" });
            const kinds = () => screen.getAllByTestId(/^tower-kind-(?!chip|only|s$)/).map((r) => r.dataset.testid);
            expect(kinds()).toEqual(["tower-kind-browsers", "tower-kind-builds", "tower-kind-shells"]);
            click("CPU");
            expect(kinds()).toEqual(["tower-kind-builds", "tower-kind-shells", "tower-kind-browsers"]);
        });

        it("lists agents biggest first, the terminals, AgentMux and the rest after them", async () => {
            const tasks = [agentTask("a-small", "Asmall"), agentTask("b-big", "Bbig"), terminalTask("term", "pwsh")];
            const owned = [
                proc(1, "x.exe", { task: "a-small", mem: GB }),
                proc(2, "y.exe", { task: "b-big", mem: 3 * GB }),
                proc(3, "z.exe", { task: "term", mem: 4 * GB }),
                proc(4, "w.exe", { mem: 5 * GB }),
            ];
            await show(hostSnap({ processes: owned, tasks }), { "tower:group": "agent" });
            const owners = () => screen.getAllByTestId(/^tower-owner-/).map((r) => r.dataset.testid);
            expect(owners()).toEqual([
                "tower-owner-b-big",
                "tower-owner-a-small",
                "tower-owner-terminals",
                "tower-owner-other",
            ]);
            click("Process");
            expect(owners()).toEqual([
                "tower-owner-a-small",
                "tower-owner-b-big",
                "tower-owner-terminals",
                "tower-owner-other",
            ]);
        });

        it("shows each row's CPU as a share of the machine, in every grouping", async () => {
            const tasks = [agentTask("block-a", "AgentX")];
            const one = [proc(1, "x.exe", { task: "block-a", kind: "shells", mem: GB, cpu: 1 })];
            await show(hostSnap({ processes: one, tasks }), { "tower:group": "agent" });
            expect(within(screen.getByTestId("tower-owner-block-a")).getByText("25%")).toBeInTheDocument();
            cleanup();
            await show(hostSnap({ processes: one, tasks }), { "tower:group": "type" });
            expect(within(screen.getByTestId("tower-kind-shells")).getByText("25%")).toBeInTheDocument();
            cleanup();
            await show(hostSnap({ processes: one, tasks }), { "tower:group": "app" });
            expect(within(table().querySelector("tbody tr") as HTMLElement).getByText("25%")).toBeInTheDocument();
            cleanup();
            await show(hostSnap({ processes: one, tasks }), { "tower:group": "none" });
            expect(within(table().querySelector("tbody tr") as HTMLElement).getByText("25%")).toBeInTheDocument();
        });

        it("names the task on a process in the flat list", async () => {
            const tasks = [agentTask("block-a", "AgentX")];
            const one = [proc(1, "x.exe", { task: "block-a" }), proc(2, "y.exe")];
            await show(hostSnap({ processes: one, tasks }), { "tower:group": "none" });
            const row = screen.getByText("x.exe").closest("tr") as HTMLElement;
            expect(row.querySelector(".tower-badge--task")).toHaveTextContent("AgentX");
            const other = screen.getByText("y.exe").closest("tr") as HTMLElement;
            expect(other.querySelector(".tower-badge--task")).toBeNull();
        });
    });

    describe("the 400-row limit", () => {
        const many = (n: number, from = 0): TowerProcess[] =>
            Array.from({ length: n }, (_, i) => {
                const k = from + i;
                // Biggest first, so the default sort keeps the order.
                return proc(1000 + k, `p${String(k).padStart(3, "0")}.exe`, { kind: "apps", mem: (1000 - k) * 1024 });
            });
        const note = () => screen.queryByText(/more not shown/);

        it("lists 400 of the flat list's rows and says how many it left out", async () => {
            await show(hostSnap({ processes: many(402) }), { "tower:group": "none" });
            expect(bodyRows()).toHaveLength(400);
            expect(screen.getByText("p000.exe")).toBeInTheDocument();
            expect(screen.getByText("p399.exe")).toBeInTheDocument();
            expect(screen.queryByText("p400.exe")).toBeNull();
            expect(note()).toHaveTextContent(/^2 more not shown: filter to narrow the list\.$/);
            expect(note()).toHaveClass("tower-muted", "tower-more");
        });

        it("says nothing at exactly 400, and counts one over as 1", async () => {
            await show(hostSnap({ processes: many(400) }), { "tower:group": "none" });
            expect(bodyRows()).toHaveLength(400);
            expect(note()).toBeNull();
            cleanup();
            await show(hostSnap({ processes: many(401) }), { "tower:group": "none" });
            expect(bodyRows()).toHaveLength(400);
            expect(note()).toHaveTextContent(/^1 more not shown: filter to narrow the list\.$/);
        });

        it("counts processes in the flat list, not apps", async () => {
            const twice = [...many(400), proc(5000, "dup.exe", { mem: 1 }), proc(5001, "dup.exe", { mem: 1 })];
            await show(hostSnap({ processes: twice }), { "tower:group": "none" });
            expect(note()).toHaveTextContent(/^2 more not shown: filter to narrow the list\.$/);
        });

        it("counts apps in the list by app, not processes", async () => {
            // 399 apps and one app of two processes: 400 groups of 401 processes.
            const apps = [
                ...many(399),
                proc(5000, "dup.exe", { kind: "apps", mem: 1 }),
                proc(5001, "dup.exe", { kind: "apps", mem: 1 }),
            ];
            await show(hostSnap({ processes: apps }), { "tower:group": "app" });
            expect(bodyRows()).toHaveLength(400);
            expect(note()).toBeNull();
        });

        it("lists 400 apps in the list by app and says how many it left out", async () => {
            await show(hostSnap({ processes: many(402) }), { "tower:group": "app" });
            expect(bodyRows()).toHaveLength(400);
            expect(screen.getByText("p000.exe")).toBeInTheDocument();
            expect(screen.getByText("p399.exe")).toBeInTheDocument();
            expect(screen.queryByText("p400.exe")).toBeNull();
            expect(note()).toHaveTextContent(/^2 more not shown: filter to narrow the list\.$/);
        });

        it("doesn't count the folded System row's processes against the limit", async () => {
            const system = Array.from({ length: 5 }, (_, i) =>
                proc(9000 + i, `sys${i}.exe`, { kind: "system", mem: 5000 * 1024 })
            );
            await show(hostSnap({ processes: [...many(400), ...system] }), { "tower:group": "none" });
            expect(note()).toBeNull();
            expect(screen.queryByText("sys0.exe")).toBeNull();
            expect(screen.getByText("p399.exe")).toBeInTheDocument();
            expect(bodyRows()).toHaveLength(401);
            cleanup();
            await show(hostSnap({ processes: [...many(400), ...system] }), { "tower:group": "app" });
            expect(note()).toBeNull();
            expect(screen.queryByText("sys0.exe")).toBeNull();
            expect(bodyRows()).toHaveLength(401);
        });

        it("has no limit note in the lists by agent or type, which fold what they hide", async () => {
            await show(hostSnap({ processes: many(405) }), { "tower:group": "agent" });
            expect(note()).toBeNull();
            cleanup();
            await show(hostSnap({ processes: many(405) }), { "tower:group": "type" });
            expect(note()).toBeNull();
        });
    });

    describe("the busiest agents and kinds open on the first list", () => {
        const nextSample = async () => {
            await vi.advanceTimersByTimeAsync(2000);
        };
        const expanded = (label: string) =>
            screen.getByRole("button", { name: new RegExp(`^(Show|Hide) ${label}`) }).getAttribute("aria-expanded");

        it("opens the two agents using the most CPU, not the two using the most memory", async () => {
            vi.useFakeTimers();
            const tasks = [agentTask("a", "Ay"), agentTask("b", "Bee"), agentTask("c", "Cee"), agentTask("d", "Dee")];
            const processes = [
                // Memory order: Bee, Dee, Cee, Ay. CPU order: Ay, Cee, Bee, Dee.
                proc(1, "ay.exe", { task: "a", mem: 1000, cpu: 3 }),
                proc(2, "bee.exe", { task: "b", mem: 4000, cpu: 1 }),
                proc(3, "cee.exe", { task: "c", mem: 2000, cpu: 2 }),
                proc(4, "dee.exe", { task: "d", mem: 3000, cpu: 0 }),
            ];
            sample.mockResolvedValue(hostSnap({ processes, tasks }));
            setMeta({ "tower:view": "processes" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Ay")).toBe("true");
            expect(expanded("Cee")).toBe("true");
            expect(expanded("Bee")).toBe("false");
            expect(expanded("Dee")).toBe("false");
            expect(screen.getByText("ay.exe")).toBeInTheDocument();
            expect(screen.queryByText("bee.exe")).toBeNull();
        });

        it("ranks an agent with no CPU figure after one using none", async () => {
            vi.useFakeTimers();
            const tasks = [agentTask("a", "Ay"), agentTask("b", "Bee"), agentTask("d", "Dee")];
            const processes = [
                proc(1, "ay.exe", { task: "a", mem: 3000, cpu: 3 }),
                proc(2, "bee.exe", { task: "b", mem: 2000, cpu: 0 }),
                proc(4, "dee.exe", { task: "d", mem: 1000 }),
            ];
            sample.mockResolvedValue(hostSnap({ processes, tasks }));
            setMeta({ "tower:view": "processes" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Ay")).toBe("true");
            expect(expanded("Bee")).toBe("true");
            expect(expanded("Dee")).toBe("false");
        });

        it("opens them even when only some agents have a CPU figure yet", async () => {
            vi.useFakeTimers();
            const tasks = [agentTask("a", "Ay"), agentTask("d", "Dee")];
            const processes = [
                proc(1, "ay.exe", { task: "a", mem: 3000, cpu: 3 }),
                proc(4, "dee.exe", { task: "d", mem: 1000 }),
            ];
            sample.mockResolvedValue(hostSnap({ processes, tasks }));
            setMeta({ "tower:view": "processes" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Ay")).toBe("true");
        });

        it("opens nothing until some agent has a CPU figure, then opens once", async () => {
            vi.useFakeTimers();
            const tasks = [agentTask("a", "Ay")];
            const first = [proc(1, "ay.exe", { task: "a", mem: 3000 })];
            sample.mockResolvedValueOnce(hostSnap({ processes: first, tasks }));
            sample.mockResolvedValue(
                hostSnap({ processes: [proc(1, "ay.exe", { task: "a", mem: 3000, cpu: 2 })], tasks })
            );
            setMeta({ "tower:view": "processes" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Ay")).toBe("false");
            await nextSample();
            expect(expanded("Ay")).toBe("true");
            // What is open is the user's from then on: closing it stays closed.
            fireEvent.click(screen.getByRole("button", { name: "Hide Ay's processes" }));
            expect(expanded("Ay")).toBe("false");
            await nextSample();
            expect(expanded("Ay")).toBe("false");
        });

        it("chooses the busiest when the list by agent is first shown, not when the first list was", async () => {
            vi.useFakeTimers();
            const tasks = [agentTask("a", "Ay"), agentTask("b", "Bee"), agentTask("c", "Cee")];
            const at = (ay: number, bee: number, cee: number) =>
                hostSnap({
                    tasks,
                    processes: [
                        proc(1, "ay.exe", { task: "a", mem: 3000, cpu: ay }),
                        proc(2, "bee.exe", { task: "b", mem: 2000, cpu: bee }),
                        proc(3, "cee.exe", { task: "c", mem: 1000, cpu: cee }),
                    ],
                });
            sample.mockResolvedValueOnce(at(3, 2, 1));
            sample.mockResolvedValue(at(0.1, 2, 5));
            setMeta({ "tower:view": "processes", "tower:group": "app" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            await nextSample();
            setMeta({ "tower:view": "processes" });
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Cee")).toBe("true");
            expect(expanded("Bee")).toBe("true");
            expect(expanded("Ay")).toBe("false");
        });

        it("opens the two busiest kinds in the list by type, the system aside", async () => {
            vi.useFakeTimers();
            const processes = [
                proc(1, "sh.exe", { kind: "shells", mem: 4000, cpu: 1 }),
                proc(2, "cargo.exe", { kind: "builds", mem: 3000, cpu: 3 }),
                proc(3, "node.exe", { kind: "runtimes", mem: 2000, cpu: 2 }),
                proc(4, "svchost.exe", { kind: "system", mem: 1000, cpu: 9 }),
                proc(5, "idle.exe", { kind: "browsers", mem: 500, cpu: 0 }),
            ];
            sample.mockResolvedValue(hostSnap({ processes }));
            setMeta({ "tower:view": "processes", "tower:group": "type" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Builds & tools")).toBe("true");
            expect(expanded("Runtimes & servers")).toBe("true");
            expect(expanded("Shells & commands")).toBe("false");
            expect(expanded("System")).toBe("false");
            expect(expanded("Browsers & web views")).toBe("false");
        });

        it("ranks a kind with no CPU figure after one using none", async () => {
            vi.useFakeTimers();
            const processes = [
                proc(1, "cargo.exe", { kind: "builds", mem: 4000, cpu: 3 }),
                proc(2, "idle.exe", { kind: "browsers", mem: 3000, cpu: 0 }),
                proc(3, "node.exe", { kind: "runtimes", mem: 2000 }),
            ];
            sample.mockResolvedValue(hostSnap({ processes }));
            setMeta({ "tower:view": "processes", "tower:group": "type" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Builds & tools")).toBe("true");
            expect(expanded("Browsers & web views")).toBe("true");
            expect(expanded("Runtimes & servers")).toBe("false");
        });

        it("opens nothing in the list by type until a kind has a CPU figure, then opens once", async () => {
            vi.useFakeTimers();
            const unrated = hostSnap({ processes: [proc(1, "cargo.exe", { kind: "builds", mem: 4000 })] });
            const rated = hostSnap({ processes: [proc(1, "cargo.exe", { kind: "builds", mem: 4000, cpu: 2 })] });
            sample.mockResolvedValueOnce(unrated);
            sample.mockResolvedValue(rated);
            setMeta({ "tower:view": "processes", "tower:group": "type" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Builds & tools")).toBe("false");
            await nextSample();
            expect(expanded("Builds & tools")).toBe("true");
            fireEvent.click(screen.getByRole("button", { name: "Hide Builds & tools" }));
            await nextSample();
            expect(expanded("Builds & tools")).toBe("false");
        });

        it("chooses the busiest kinds when the list by type is first shown", async () => {
            vi.useFakeTimers();
            const at = (builds: number, runtimes: number, shells: number) =>
                hostSnap({
                    processes: [
                        proc(1, "cargo.exe", { kind: "builds", mem: 3000, cpu: builds }),
                        proc(2, "node.exe", { kind: "runtimes", mem: 2000, cpu: runtimes }),
                        proc(3, "sh.exe", { kind: "shells", mem: 1000, cpu: shells }),
                    ],
                });
            sample.mockResolvedValueOnce(at(3, 2, 1));
            sample.mockResolvedValue(at(0.1, 2, 5));
            setMeta({ "tower:view": "processes", "tower:group": "app" });
            renderTower();
            await vi.advanceTimersByTimeAsync(0);
            await nextSample();
            setMeta({ "tower:view": "processes", "tower:group": "type" });
            await vi.advanceTimersByTimeAsync(0);
            expect(expanded("Shells & commands")).toBe("true");
            expect(expanded("Runtimes & servers")).toBe("true");
            expect(expanded("Builds & tools")).toBe("false");
        });
    });
});
