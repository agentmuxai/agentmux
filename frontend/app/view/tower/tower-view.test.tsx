// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const sample = vi.fn();
const reveal = vi.fn();
const peersCmd = vi.fn();
const pairCmd = vi.fn();
const forgetCmd = vi.fn();
const setConfig = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        TowerSampleCommand: (...args: unknown[]) => sample(...args),
        TowerPeersCommand: (...args: unknown[]) => peersCmd(...args),
        TowerPairCommand: (...args: unknown[]) => pairCmd(...args),
        TowerForgetCommand: (...args: unknown[]) => forgetCmd(...args),
        SetConfigCommand: (...args: unknown[]) => setConfig(...args),
    },
}));
vi.mock("@/store/global", () => ({ settingsAtom: () => ({}) }));
const studio = { connection: "peer:p1", hostname: "studio", address: "198.51.100.20:47900", paired_ms: 1 };
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: (...args: unknown[]) => reveal(...args) }));
const remoteRecords = [
    { name: "build-box", kind: "ssh", platform: { os: "linux", arch: "x86_64" } },
    { name: "mac-mini", kind: "ssh", platform: { os: "macos", arch: "arm64" } },
    { name: "pi", kind: "ssh", platform: { os: "linux", arch: "armv7l" }, helper: { state: "unsupported" } },
    { name: "wsl://Ubuntu", kind: "wsl", platform: null },
    { name: "win-server", kind: "ssh", platform: { os: "mingw64_nt-10.0", arch: "x86_64" } },
];
vi.mock("@/app/store/remotes-store", () => ({ remotesList: () => () => remoteRecords }));
// An agent's color comes from its pane's meta; the tests' panes all have one.
vi.mock("@/app/store/mos", () => ({
    makeORef: (t: string, id: string) => `${t}:${id}`,
    getObjectValue: () => ({ meta: { "frame:activebordercolor": "#ff8800" } }),
}));
vi.mock("@/app/block/pane-identity", () => ({
    blockRoleColor: (meta: Record<string, unknown> | undefined) => meta?.["frame:activebordercolor"],
    isLightThemeActive: () => false,
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { TowerSnapshot } from "@/app/store/rpc-api";
import { chartKey } from "./tower-memory";
import { TowerViewModel } from "./tower-model";
import { TowerView } from "./tower-view";

const GB = 1024 * 1024 * 1024;

function snapshot(host = false): TowerSnapshot {
    return {
        ts_ms: 1,
        hostname: "narko",
        os: "windows",
        cpu_count: 4,
        memory_metric: "private working set",
        interval_ms: 2000,
        remote: false,
        tasks: [
            {
                id: "block-a",
                kind: "agent",
                label: "AgentX",
                tracking: "high",
                cpu: 2,
                cpu_account: true,
                mem: 1 * GB,
                processes: [
                    {
                        id: "10:1",
                        pid: 10,
                        name: "claude.exe",
                        cpu: 0.5,
                        mem: GB / 2,
                        role: "main",
                        started_at_ms: 100,
                    },
                    {
                        id: "11:1",
                        pid: 11,
                        ppid: 10,
                        name: "node.exe",
                        cpu: 1.5,
                        mem: GB / 2,
                        role: "started",
                        started_at_ms: 200,
                    },
                ],
            },
            {
                id: "agentmux",
                kind: "agentmux",
                label: "AgentMux",
                tracking: "tree",
                cpu: 0.1,
                cpu_account: false,
                mem: 2 * GB,
                processes: [
                    {
                        id: "5:1",
                        pid: 5,
                        name: "agentmux-srv.exe",
                        cpu: 0.1,
                        mem: 2 * GB,
                        role: "main",
                        detail: "Server",
                    },
                ],
            },
        ],
        // The machine: 3.3 cores busy and 8 GB, so 1.2 cores and 5 GB are
        // everything else's.
        machine: { cpu: 3.3, mem: 8 * GB, processes: 40 },
        host: host
            ? {
                  processes: [
                      { id: "10:1", pid: 10, name: "claude.exe", cpu: 0.5, mem: GB / 2, task: "block-a" },
                      { id: "4:1", pid: 4, name: "System", cpu: 0.01, mem: 1024 },
                      { id: "77:1", pid: 77, name: "WindowServer" },
                  ],
                  unmeasured: 1,
                  cpu: 0.51,
                  mem: GB / 2 + 1024,
                  total: 3,
                  matched: 3,
              }
            : undefined,
    };
}

const [meta, setMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant" | "windowHidden">("active");
const setMetaMock = vi.fn((patch: Record<string, unknown>) => {
    setMeta((prev) => {
        const next = { ...prev, ...patch };
        for (const [k, v] of Object.entries(patch)) if (v === null) delete next[k];
        return next;
    });
    return Promise.resolve();
});

/** The pane's meta as the tests set it. Tower opens on Memory, and almost
 *  every test is about another view, so a test that doesn't name its view is
 *  on Agents, as it was before the Memory view; `renderTower({ rawMeta: true })`
 *  is the pane with nothing set. */
const withAgentsView = () => ({ "tower:view": "agents", ...meta() });

function renderTower(opts: { rawMeta?: boolean } = {}) {
    return render(() => {
        const model = new TowerViewModel({
            blockId: "tower-1",
            meta: (opts.rawMeta ? meta : withAgentsView) as unknown as PaneTabHostContext["meta"],
            setMeta: setMetaMock,
            isFocused: () => true,
            visibility,
        });
        return <TowerView model={model} />;
    });
}

describe("Tower", () => {
    beforeEach(() => {
        peersCmd.mockResolvedValue({ peers: [studio] });
        sample.mockImplementation((_client: unknown, req: { host: boolean }) => Promise.resolve(snapshot(req.host)));
    });
    afterEach(() => {
        cleanup();
        vi.clearAllMocks();
        vi.useRealTimers();
        setMeta({});
        setVisibility("active");
    });

    it("the Agents view lists each agent, then AgentMux and everything else, with their CPU and memory", async () => {
        renderTower();
        const agent = await screen.findByTestId("tower-rail-block-a");
        // 2 cores busy of 4: 50%.
        expect(within(agent).getByText("50%")).toBeInTheDocument();
        expect(within(agent).getByText("1.0 GB")).toBeInTheDocument();
        expect(within(agent).getByText("AgentX")).toBeInTheDocument();
        const rows = screen.getAllByTestId(/^tower-rail-/).map((r) => r.dataset.testid);
        expect(rows).toEqual(["tower-rail-block-a", "tower-rail-agentmux", "tower-rail-other"]);
        // Everything else: the machine less every task (3.3 - 2.1 cores, 8 - 3 GB).
        const other = screen.getByTestId("tower-rail-other");
        expect(within(other).getByText("30%")).toBeInTheDocument();
        expect(within(other).getByText("5.0 GB")).toBeInTheDocument();
        expect(sample).toHaveBeenCalledWith(expect.anything(), { host: false });
    });

    it("shows CPU as a share of the whole machine, with no switch to a share of one core", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.queryByRole("radio", { name: "% of a core" })).toBeNull();
        expect(screen.queryByRole("radiogroup", { name: "Show CPU as" })).toBeNull();
        // Block a's 2 cores busy of 4 are 50% of the machine, never 200%.
        expect(within(screen.getByTestId("tower-rail-block-a")).getByText("50%")).toBeInTheDocument();
        expect(screen.queryByText("200%")).toBeNull();
    });

    it("the first agent is selected: its processes as a tree, a parent with everything it started", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.getByTestId("tower-rail-block-a")).toHaveAttribute("aria-selected", "true");
        const claude = screen.getByText("claude.exe").closest("tr")!;
        // claude.exe's own 0.5 core plus node.exe's 1.5: 2 of 4 cores.
        expect(within(claude).getByText("50%")).toBeInTheDocument();
        const node = screen.getByText("node.exe").closest("tr")!;
        // 1.5 cores of 4.
        expect(within(node).getByText("38%")).toBeInTheDocument();
        fireEvent.click(within(claude).getByRole("button", { name: "Hide what it started" }));
        expect(screen.queryByText("node.exe")).toBeNull();
        expect(screen.queryByRole("button", { name: /command line/i })).toBeNull();
    });

    it("selecting an entry shows it, and is kept with the pane", async () => {
        renderTower();
        fireEvent.click(await screen.findByTestId("tower-rail-agentmux"));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:agent": "agentmux" });
        expect(screen.getByTestId("tower-rail-agentmux")).toHaveAttribute("aria-selected", "true");
        expect(screen.getByText("agentmux-srv.exe")).toBeInTheDocument();
        expect(screen.queryByText("claude.exe")).toBeNull();
        // Arrow keys move the selection.
        fireEvent.keyDown(screen.getByRole("listbox", { name: "Agents" }), { key: "ArrowDown" });
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:agent": "other" });
    });

    it("everything else points to the Processes view", async () => {
        setMeta({ "tower:agent": "other" });
        renderTower();
        expect(await screen.findByText(/37 processes AgentMux didn't start/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Show in Processes" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:view": "processes", "tower:only": "other" });
    });

    it("terminals are one entry, each terminal heading its processes", async () => {
        const snap = snapshot();
        snap.tasks.push(
            {
                id: "term-1",
                kind: "terminal",
                label: "pwsh",
                tracking: "high",
                cpu: 0.25,
                cpu_account: true,
                mem: GB,
                processes: [{ id: "30:1", pid: 30, name: "pwsh.exe", cpu: 0.25, mem: GB, role: "main" }],
            },
            {
                id: "term-2",
                kind: "terminal",
                label: "bash",
                tracking: "high",
                cpu: 0.25,
                cpu_account: true,
                mem: GB,
                processes: [{ id: "31:1", pid: 31, name: "bash.exe", cpu: 0.25, mem: GB, role: "main" }],
            }
        );
        sample.mockResolvedValue(snap);
        setMeta({ "tower:agent": "terminals" });
        renderTower();
        const terminals = await screen.findByTestId("tower-rail-terminals");
        // Half a core of 4, and 2 GB, together.
        expect(within(terminals).getByText("13%")).toBeInTheDocument();
        expect(within(terminals).getByText("2.0 GB")).toBeInTheDocument();
        expect(screen.getByTestId("tower-task-term-1")).toHaveTextContent("pwsh");
        expect(screen.getByText("bash.exe")).toBeInTheDocument();
    });

    it("folds same-named processes under one line: rustc.exe ×3", async () => {
        const snap = snapshot();
        for (const pid of [40, 41, 42]) {
            snap.tasks[0].processes.push({
                id: `${pid}:1`,
                pid,
                ppid: 11,
                name: "rustc.exe",
                cpu: 1,
                mem: GB,
                started_at_ms: 300,
            });
        }
        sample.mockResolvedValue(snap);
        renderTower();
        const many = await screen.findByTestId("tower-many-rustc.exe");
        expect(within(many).getByText("×3")).toBeInTheDocument();
        // 3 cores of 4.
        expect(within(many).getByText("75%")).toBeInTheDocument();
        expect(screen.getAllByText("rustc.exe")).toHaveLength(1);
        fireEvent.click(within(many).getByRole("button", { name: "Show the rustc.exe processes" }));
        expect(screen.getAllByText("rustc.exe")).toHaveLength(4);
    });

    it("an ×n line opens on its own, not with a same-named one under another parent", async () => {
        const snap = snapshot();
        const procs = snap.tasks[0].processes;
        procs.push(
            { id: "50:1", pid: 50, ppid: 11, name: "cargo.exe", cpu: 0, mem: GB, started_at_ms: 300 },
            { id: "60:1", pid: 60, ppid: 11, name: "cargo.exe", cpu: 0, mem: GB, started_at_ms: 300 }
        );
        for (const [pid, ppid] of [
            [51, 50],
            [52, 50],
            [61, 60],
            [62, 60],
        ]) {
            procs.push({ id: `${pid}:1`, pid, ppid, name: "rustc.exe", cpu: 1, mem: GB, started_at_ms: 400 });
        }
        sample.mockResolvedValue(snap);
        renderTower();
        await waitFor(() => expect(screen.getAllByTestId("tower-many-rustc.exe")).toHaveLength(2));
        const [first] = screen.getAllByTestId("tower-many-rustc.exe");
        fireEvent.click(within(first).getByRole("button", { name: "Show the rustc.exe processes" }));
        // Two from the opened line, and the other line still closed.
        expect(screen.getAllByText("rustc.exe")).toHaveLength(4);
        expect(screen.getAllByRole("button", { name: "Show the rustc.exe processes" })).toHaveLength(1);
    });

    it("says what each of AgentMux's own processes is, with its executable alongside", async () => {
        renderTower();
        fireEvent.click(await screen.findByTestId("tower-rail-agentmux"));
        const srv = screen.getByText("Server").closest("tr")!;
        expect(within(srv).getByText("agentmux-srv.exe")).toHaveClass("tower-muted", "tower-exe");
    });

    it("a process that exited in the last minute stays in the tree, dimmed, with what it used", async () => {
        const snap = snapshot();
        snap.ts_ms = 10_000;
        snap.tasks[0].exited = [40, 41, 42].map((pid) => ({
            id: `${pid}:1`,
            pid,
            ppid: 11,
            name: "rustc.exe",
            role: "started" as const,
            cpu_time_ns: 2e9,
            peak_mem: GB,
            exited_ms: 6_000,
            started_at_ms: 300,
        }));
        snap.tasks[0].exited.push({
            id: "43:1",
            pid: 43,
            ppid: 11,
            name: "cc.exe",
            cpu_time_ns: 1e9,
            exited_ms: 6_000,
            started_at_ms: 300,
        });
        sample.mockResolvedValue(snap);
        renderTower();
        const gone = await screen.findByTestId("tower-many-rustc.exe-exited");
        expect(gone).toHaveClass("tower-process-row--exited");
        expect(within(gone).getByText("×3")).toBeInTheDocument();
        expect(within(gone).getByText("exited")).toBeInTheDocument();
        const cc = screen.getByText("cc.exe").closest("tr")!;
        expect(cc).toHaveClass("tower-process-row--exited");
        expect(within(cc).getByText("4 s ago")).toHaveAttribute("title", "Exited 4 s ago");
        // Right after the name, not at the far edge of the name cell.
        expect(within(cc).getByText("4 s ago").parentElement).toHaveClass("tower-name-tail");
        // It isn't counted as running.
        expect(within(screen.getByTestId("tower-rail-block-a")).getByText("50%")).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: "More columns" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:cols": "more" });
        expect(screen.getByRole("columnheader", { name: "CPU time" })).toBeInTheDocument();
        // The three exited compilers' 6 s, and their peak.
        expect(within(gone).getByText("6.0 s")).toBeInTheDocument();
        expect(within(gone).getByText("1.0 GB")).toBeInTheDocument();
    });

    it("the process a tool call runs says which call, in both views", async () => {
        const snap = snapshot(true);
        snap.tasks[0].processes[1].started_by = "Run the srv tests";
        snap.host!.processes.push({
            id: "11:1",
            pid: 11,
            name: "node.exe",
            cpu: 1.5,
            mem: GB / 2,
            task: "block-a",
            started_by: "Run the srv tests",
        });
        sample.mockResolvedValue(snap);
        renderTower();
        const row = (await screen.findByText("node.exe")).closest("tr")!;
        expect(within(row).getByText("⟵ Run the srv tests")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("tab", { name: "Processes" }));
        await waitFor(() => expect(screen.getAllByText("⟵ Run the srv tests").length).toBeGreaterThan(0));
    });

    it("the selected agent's last 10 minutes: CPU and memory charts, once there are two samples", async () => {
        vi.useFakeTimers();
        let ts = 1_000_000;
        sample.mockImplementation(() => {
            const snap = snapshot();
            snap.ts_ms = ts;
            ts += 2000;
            return Promise.resolve(snap);
        });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        // One sample: no chart yet.
        expect(screen.queryByTestId("tower-history")).toBeNull();
        await vi.advanceTimersByTimeAsync(2000);
        const charts = screen.getAllByRole("img", { name: /over the last 10 minutes/ });
        expect(charts.map((c) => c.getAttribute("aria-label"))).toEqual([
            "Memory over the last 10 minutes: now 1.0 GB, peak 1.0 GB",
            "CPU over the last 10 minutes: now 50%, peak 50%",
        ]);
        // Under the pointer: when, and how much.
        fireEvent.pointerMove(charts[1], { clientX: 10_000 });
        expect(screen.getByTestId("tower-chart-tip")).toHaveTextContent("0 s ago · 50%");
        fireEvent.pointerLeave(charts[1]);
        expect(screen.queryByTestId("tower-chart-tip")).toBeNull();
    });

    it("a refresh that brings the same measurement adds no point to the history", async () => {
        vi.useFakeTimers();
        sample.mockImplementation(() => Promise.resolve(snapshot()));
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        await vi.advanceTimersByTimeAsync(2000);
        await vi.advanceTimersByTimeAsync(2000);
        // Every sample had the same time: still one point, so no chart.
        expect(screen.queryByTestId("tower-history")).toBeNull();
    });

    it("keeps each row, and updates it, across a refresh", async () => {
        vi.useFakeTimers();
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        const node = screen.getByText("node.exe").closest("tr")!;
        // The next poll brings new numbers in new objects.
        sample.mockImplementation(() => {
            const next = snapshot();
            next.tasks[0].processes[1].cpu = 1.0;
            return Promise.resolve(next);
        });
        await vi.advanceTimersByTimeAsync(2000);
        expect(sample).toHaveBeenCalledTimes(2);
        // Same row element, updated in place (not rebuilt every refresh).
        expect(screen.getByText("node.exe").closest("tr")).toBe(node);
        expect(within(node).getByText("25%")).toBeInTheDocument();
    });

    it("a label that arrives on a later sample shows on the row already there", async () => {
        vi.useFakeTimers();
        sample.mockImplementation(() => {
            const first = snapshot();
            delete first.tasks[1].processes[0].detail;
            return Promise.resolve(first);
        });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        fireEvent.click(screen.getByTestId("tower-rail-agentmux"));
        expect(screen.getByText("agentmux-srv.exe")).toBeInTheDocument();
        expect(screen.queryByText("Server")).toBeNull();
        sample.mockImplementation(() => Promise.resolve(snapshot()));
        await vi.advanceTimersByTimeAsync(2000);
        expect(screen.getByText("Server")).toBeInTheDocument();
    });

    it("reveals an agent's pane, and offers no pane for AgentMux itself", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        fireEvent.click(screen.getByRole("button", { name: "Show this pane" }));
        expect(reveal).toHaveBeenCalledWith("block-a", { source: "tower" });
        fireEvent.click(screen.getByTestId("tower-rail-agentmux"));
        await waitFor(() => expect(screen.queryByRole("button", { name: "Show this pane" })).toBeNull());
    });

    it("the Processes view lists every process, filters, and says what it can't measure", async () => {
        // "host" and "off" are how the view and a flat list were saved before
        // they were renamed.
        setMeta({ "tower:view": "host", "tower:group": "off" });
        renderTower();
        expect(await screen.findByText("WindowServer")).toBeInTheDocument();
        expect(sample).toHaveBeenCalledWith(expect.anything(), { host: true });
        expect(
            screen.getByText(/1 owned by other users can't be measured without administrator rights/)
        ).toBeInTheDocument();
        // The unmeasured row shows dashes, never zeros.
        const ws = screen.getByText("WindowServer").closest("tr")!;
        expect(within(ws).getAllByText("—")).toHaveLength(2);
        // A process's task is named.
        expect(within(screen.getByText("claude.exe").closest("tr")!).getByText("AgentX")).toBeInTheDocument();

        fireEvent.input(screen.getByTestId("tower-filter-input"), { target: { value: "agentx" } });
        await waitFor(() => expect(screen.queryByText("System")).toBeNull());
        expect(screen.getByText("claude.exe")).toBeInTheDocument();
    });

    it("lists the machines, and can't pick an SSH host the helper doesn't run on", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        const picker = screen.getByRole("combobox", { name: "Machine" }) as HTMLSelectElement;
        const options = Array.from(picker.options).map((o) => [o.value, o.disabled]);
        expect(options).toEqual([
            ["", false],
            ["peer:p1", false],
            ["build-box", false],
            ["mac-mini", false],
            ["pi", true],
            ["wsl://Ubuntu", false],
            ["win-server", true],
            ["__pair__", false],
        ]);
        fireEvent.change(picker, { target: { value: "build-box" } });
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:connection": "build-box" });
    });

    it("another machine: its processes only, asked for with the pane's id, no command lines", async () => {
        vi.useFakeTimers();
        sample.mockImplementation((_c: unknown, req: { connection?: string; filter?: string }) =>
            Promise.resolve({
                ...snapshot(),
                remote: true,
                hostname: req.connection,
                tasks: [],
                host: {
                    processes: [{ id: "77:1", pid: 77, name: req.filter ? "sshd" : "cargo", cpu: 1.25, mem: GB }],
                    unmeasured: 0,
                    cpu: 2,
                    mem: 8 * GB,
                    total: 410,
                    matched: req.filter ? 2 : 410,
                },
            })
        );
        setMeta({ "tower:connection": "build-box" });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenLastCalledWith(
            expect.anything(),
            { host: true, connection: "build-box", filter: "", block_id: "tower-1" },
            { timeout: 180000 }
        );
        expect(screen.queryByRole("tab", { name: "Agents" })).toBeNull();
        const cargo = screen.getByText("cargo").closest("tr")!;
        expect(within(cargo).queryByRole("button", { name: "Show command line" })).toBeNull();
        expect(screen.getByText(/410 processes/)).toBeInTheDocument();
        expect(screen.getByText(/showing the 1 busiest and largest/)).toBeInTheDocument();

        // Its filter is applied there, once typing pauses.
        fireEvent.input(screen.getByTestId("tower-filter-input"), { target: { value: "ssh" } });
        await vi.advanceTimersByTimeAsync(300);
        expect(sample).toHaveBeenLastCalledWith(
            expect.anything(),
            expect.objectContaining({ connection: "build-box", filter: "ssh" }),
            expect.anything()
        );
        await vi.advanceTimersByTimeAsync(0);
        expect(screen.getByText(/of 2 matching/)).toBeInTheDocument();
    });

    it("another machine that failed isn't asked again until Retry", async () => {
        vi.useFakeTimers();
        sample.mockRejectedValueOnce(new Error("no helper for armv7l"));
        setMeta({ "tower:connection": "build-box" });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        expect(screen.getByRole("alert").textContent).toContain("armv7l");
        await vi.advanceTimersByTimeAsync(30_000);
        expect(sample).toHaveBeenCalledTimes(1);
        // Hiding and showing the pane, or a new filter, doesn't try again.
        setVisibility("dormant");
        await vi.advanceTimersByTimeAsync(0);
        setVisibility("active");
        await vi.advanceTimersByTimeAsync(1000);
        expect(sample).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByRole("button", { name: "Retry" }));
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(2);
    });

    it("a new filter waits for the request already in flight", async () => {
        vi.useFakeTimers();
        const remote = () => ({ ...snapshot(true), remote: true, tasks: [] });
        sample.mockImplementation(() => Promise.resolve(remote()));
        setMeta({ "tower:connection": "build-box" });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(1);
        // The next poll is slow to answer.
        let answer: (s: TowerSnapshot) => void = () => {};
        sample.mockImplementationOnce(() => new Promise((r) => (answer = r)));
        await vi.advanceTimersByTimeAsync(2000);
        expect(sample).toHaveBeenCalledTimes(2);
        fireEvent.input(screen.getByTestId("tower-filter-input"), { target: { value: "ssh" } });
        await vi.advanceTimersByTimeAsync(1000);
        expect(sample).toHaveBeenCalledTimes(2);
        answer(remote());
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(3);
        expect(sample).toHaveBeenLastCalledWith(
            expect.anything(),
            expect.objectContaining({ filter: "ssh" }),
            expect.anything()
        );
    });

    it("switching machines doesn't wait for another machine's slow request", async () => {
        vi.useFakeTimers();
        sample.mockImplementationOnce(() => new Promise(() => {}));
        setMeta({ "tower:connection": "build-box" });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(1);
        setMeta({});
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(2);
        expect(sample).toHaveBeenLastCalledWith(expect.anything(), { host: false });
    });

    it("pairs with another AgentMux computer from its link, and shows it", async () => {
        pairCmd.mockResolvedValue({ ...studio, connection: "peer:p2", hostname: "laptop" });
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        fireEvent.change(screen.getByRole("combobox", { name: "Machine" }), { target: { value: "__pair__" } });
        const form = screen.getByTestId("tower-pair");
        const pair = within(form).getByRole("button", { name: "Pair" });
        expect(pair).toBeDisabled();
        fireEvent.input(within(form).getByLabelText("Pairing link"), {
            target: { value: " agentmux://pair?v=1&code=X " },
        });
        fireEvent.click(pair);
        await waitFor(() => expect(setMetaMock).toHaveBeenCalledWith({ "tower:connection": "peer:p2" }));
        expect(pairCmd).toHaveBeenCalledWith(
            expect.anything(),
            { link: "agentmux://pair?v=1&code=X" },
            expect.anything()
        );
        expect(screen.queryByTestId("tower-pair")).toBeNull();
    });

    it("says why pairing failed and keeps the form", async () => {
        pairCmd.mockRejectedValue(new Error("The code was refused"));
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        fireEvent.change(screen.getByRole("combobox", { name: "Machine" }), { target: { value: "__pair__" } });
        fireEvent.input(screen.getByLabelText("Pairing link"), { target: { value: "agentmux://pair?v=1" } });
        fireEvent.click(screen.getByRole("button", { name: "Pair" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("The code was refused");
        expect(screen.getByTestId("tower-pair")).toBeInTheDocument();
    });

    it("a paired AgentMux computer shows its agents, but none of its panes can be revealed here", async () => {
        sample.mockImplementation(() => Promise.resolve({ ...snapshot(true), remote: true, hostname: "studio" }));
        setMeta({ "tower:connection": "peer:p1" });
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.getByRole("tab", { name: "Agents" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Show this pane" })).toBeNull();
        expect(sample).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ connection: "peer:p1", host: false }),
            expect.anything()
        );
    });

    it("forgets a paired computer and goes back to this one", async () => {
        forgetCmd.mockResolvedValue({ peers: [] });
        setMeta({ "tower:connection": "peer:p1" });
        renderTower();
        fireEvent.click(await screen.findByRole("button", { name: "Forget this computer" }));
        await waitFor(() =>
            expect(forgetCmd).toHaveBeenCalledWith(expect.anything(), { connection: "peer:p1" }, expect.anything())
        );
        await waitFor(() => expect(setMetaMock).toHaveBeenCalledWith({ "tower:connection": null }));
    });

    it("says why a paired computer couldn't be forgotten", async () => {
        forgetCmd.mockRejectedValue(new Error("Couldn't remove the pairing with studio from the keychain"));
        setMeta({ "tower:connection": "peer:p1" });
        renderTower();
        fireEvent.click(await screen.findByRole("button", { name: "Forget this computer" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("from the keychain");
        expect(setMetaMock).not.toHaveBeenCalledWith({ "tower:connection": null });
    });

    it("the Processes view groups processes of one app, and can list them flat", async () => {
        const many = snapshot(true);
        many.host!.processes = [
            { id: "20:1", pid: 20, name: "chrome.exe", cpu: 0.5, mem: GB },
            { id: "21:1", pid: 21, name: "chrome.exe", cpu: 0.25, mem: GB },
            { id: "4:1", pid: 4, name: "System", cpu: 0.01, mem: 1024 },
        ];
        sample.mockResolvedValue(many);
        setMeta({ "tower:view": "processes", "tower:group": "app" });
        renderTower();
        const chrome = await screen.findByTestId("tower-app-chrome.exe");
        expect(within(chrome).getByText("(2)")).toBeInTheDocument();
        expect(within(chrome).getByText("2.0 GB")).toBeInTheDocument();
        // 0.75 of a core on a 4-core machine.
        expect(within(chrome).getByText("19%")).toBeInTheDocument();
        expect(screen.getAllByText("chrome.exe")).toHaveLength(1);
        fireEvent.click(within(chrome).getByRole("button", { name: "Show processes" }));
        expect(screen.getAllByText("chrome.exe")).toHaveLength(3);
        // A group of one is a plain row.
        expect(screen.queryByTestId("tower-app-system")).toBeNull();
        expect(screen.getByText("System")).toBeInTheDocument();

        fireEvent.click(screen.getByRole("radio", { name: "None" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:group": "none" });
        await waitFor(() => expect(screen.queryByTestId("tower-app-chrome.exe")).toBeNull());
        expect(screen.getAllByText("chrome.exe")).toHaveLength(2);
    });

    it("the Processes view groups every process under its agent, the busiest open", async () => {
        const snap = snapshot(true);
        snap.host!.processes.push({ id: "20:1", pid: 20, name: "chrome.exe", cpu: 0.2, mem: GB });
        sample.mockResolvedValue(snap);
        setMeta({ "tower:view": "processes" });
        renderTower();
        const agent = await screen.findByTestId("tower-owner-block-a");
        expect(within(agent).getByText("AgentX")).toBeInTheDocument();
        expect(within(agent).getByText("(1)")).toBeInTheDocument();
        const owners = screen.getAllByTestId(/^tower-owner-/).map((r) => r.dataset.testid);
        expect(owners).toEqual(["tower-owner-block-a", "tower-owner-other"]);
        // The busiest agent starts open; the other processes don't.
        await waitFor(() => expect(screen.getByText("claude.exe")).toBeInTheDocument());
        expect(screen.queryByText("chrome.exe")).toBeNull();
        expect(agent.getAttribute("style")).toContain("--tower-owner-color");
        // Other processes open to their apps.
        fireEvent.click(
            within(screen.getByTestId("tower-owner-other")).getByRole("button", { name: /Show Other processes/ })
        );
        expect(screen.getByText("chrome.exe")).toBeInTheDocument();
        expect(screen.getByText("WindowServer")).toBeInTheDocument();
    });

    it("a search shows each match under its owner, and only owners with a match", async () => {
        setMeta({ "tower:view": "processes" });
        renderTower();
        await screen.findByTestId("tower-owner-block-a");
        fireEvent.input(screen.getByTestId("tower-filter-input"), { target: { value: "system" } });
        await waitFor(() => expect(screen.queryByTestId("tower-owner-block-a")).toBeNull());
        // The match's group opens by itself while the search is on.
        expect(screen.getByTestId("tower-owner-other")).toBeInTheDocument();
        expect(screen.getByText("System")).toBeInTheDocument();
    });

    it("the busiest agents are chosen once their CPU is known, not from the first sample's names", async () => {
        vi.useFakeTimers();
        const withAgents = (rated: boolean) => {
            const snap = snapshot(true);
            snap.tasks.unshift(
                {
                    id: "a-quiet",
                    kind: "agent",
                    label: "Aquiet",
                    tracking: "high",
                    cpu: rated ? 0 : undefined,
                    cpu_account: true,
                    mem: 1,
                    processes: [],
                },
                {
                    id: "b-idle",
                    kind: "agent",
                    label: "Bidle",
                    tracking: "high",
                    cpu: rated ? 0 : undefined,
                    cpu_account: true,
                    mem: 1,
                    processes: [],
                }
            );
            snap.host!.processes.push(
                { id: "90:1", pid: 90, name: "quiet.exe", cpu: rated ? 0 : undefined, mem: 1, task: "a-quiet" },
                { id: "91:1", pid: 91, name: "idle.exe", cpu: rated ? 0.01 : undefined, mem: 1, task: "b-idle" },
                { id: "92:1", pid: 92, name: "build.exe", cpu: rated ? 3 : undefined, mem: 1, task: "block-a" }
            );
            // The first sample: no process has a rate yet.
            if (!rated) snap.host!.processes.forEach((p) => (p.cpu = undefined));
            return snap;
        };
        sample.mockResolvedValueOnce(withAgents(false)).mockResolvedValue(withAgents(true));
        setMeta({ "tower:view": "processes" });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        // No rates yet: nothing opened by name.
        expect(screen.queryByText("quiet.exe")).toBeNull();
        await vi.advanceTimersByTimeAsync(2000);
        // AgentX (3 cores) and Bidle are the busiest two; Aquiet stays shut.
        expect(screen.getByText("build.exe")).toBeInTheDocument();
        expect(screen.getByText("idle.exe")).toBeInTheDocument();
        expect(screen.queryByText("quiet.exe")).toBeNull();
    });

    it("while searching, a group's arrow can't close it, and an app under Other opens", async () => {
        const snap = snapshot(true);
        snap.host!.processes.push(
            { id: "20:1", pid: 20, name: "rustc.exe", cpu: 0.2, mem: GB },
            { id: "21:1", pid: 21, name: "rustc.exe", cpu: 0.2, mem: GB }
        );
        sample.mockResolvedValue(snap);
        setMeta({ "tower:view": "processes" });
        renderTower();
        await screen.findByTestId("tower-owner-other");
        fireEvent.input(screen.getByTestId("tower-filter-input"), { target: { value: "rustc" } });
        await waitFor(() => expect(screen.getAllByText("rustc.exe")).toHaveLength(3));
        expect(within(screen.getByTestId("tower-owner-other")).getByRole("button")).toBeDisabled();
        expect(within(screen.getByTestId("tower-app-rustc.exe")).getByRole("button")).toBeDisabled();
    });

    it("groups by agent, app or nothing; another machine has no agents to group by", async () => {
        setMeta({ "tower:view": "processes" });
        renderTower();
        await screen.findByTestId("tower-owner-block-a");
        const radios = screen.getAllByRole("radio").map((r) => r.textContent);
        expect(radios).toEqual(expect.arrayContaining(["Agent", "App", "None"]));
        fireEvent.click(screen.getByRole("radio", { name: "App" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:group": "app" });
        fireEvent.click(screen.getByRole("radio", { name: "Agent" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:group": null });
        cleanup();

        sample.mockImplementation(() => Promise.resolve({ ...snapshot(true), remote: true, tasks: [] }));
        setMeta({ "tower:connection": "build-box" });
        renderTower();
        await screen.findByText("System");
        expect(screen.queryByRole("radio", { name: "Agent" })).toBeNull();
    });

    it("going from an agent to Processes shows only that agent's processes, until Show all", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        fireEvent.click(screen.getByRole("tab", { name: "Processes" }));
        // The agent the Agents view showed (the first, none chosen) goes along.
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:view": "processes", "tower:only": "block-a" });
        expect(await screen.findByTestId("tower-only")).toHaveTextContent("Only AgentX");
        await waitFor(() => expect(screen.getByText("claude.exe")).toBeInTheDocument());
        expect(screen.queryByTestId("tower-owner-other")).toBeNull();
        expect(screen.queryByText("System")).toBeNull();

        fireEvent.click(within(screen.getByTestId("tower-only")).getByRole("button", { name: "Show all" }));
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:only": null });
        expect(await screen.findByTestId("tower-owner-other")).toBeInTheDocument();
        expect(screen.queryByTestId("tower-only")).toBeNull();
    });

    it("the filter is dropped on another machine and back on the Agents view", async () => {
        setMeta({ "tower:view": "processes", "tower:only": "block-a" });
        renderTower();
        await screen.findByTestId("tower-only");
        fireEvent.change(screen.getByRole("combobox", { name: "Machine" }), { target: { value: "peer:p1" } });
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:connection": "peer:p1", "tower:only": null });
        setMeta({ "tower:view": "processes", "tower:only": "block-a" });
        fireEvent.click(await screen.findByRole("tab", { name: "Agents" }));
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:view": "agents", "tower:only": null });
    });

    it("a pane opened on Processes isn't filtered, and switching within it carries nothing", async () => {
        setMeta({ "tower:view": "processes" });
        renderTower();
        await screen.findByTestId("tower-owner-other");
        expect(screen.queryByTestId("tower-only")).toBeNull();
        fireEvent.click(screen.getByRole("tab", { name: "Processes" }));
        expect(setMetaMock).not.toHaveBeenCalledWith(expect.objectContaining({ "tower:only": expect.anything() }));
    });

    it("polls only while visible", async () => {
        vi.useFakeTimers();
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(2000);
        expect(sample).toHaveBeenCalledTimes(2);
        setVisibility("dormant");
        await vi.advanceTimersByTimeAsync(10_000);
        expect(sample).toHaveBeenCalledTimes(2);
        setVisibility("active");
        await vi.advanceTimersByTimeAsync(0);
        expect(sample).toHaveBeenCalledTimes(3);
    });

    it("asks again after a second when the first sample has no CPU rates yet", async () => {
        vi.useFakeTimers();
        const first = snapshot();
        first.tasks.forEach((t) => (t.cpu = undefined));
        sample.mockResolvedValueOnce(first);
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        await vi.advanceTimersByTimeAsync(1000);
        expect(sample).toHaveBeenCalledTimes(2);
    });

    it("shows an error and keeps trying", async () => {
        vi.useFakeTimers();
        sample.mockRejectedValueOnce(new Error("boom"));
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        expect(screen.getByRole("alert").textContent).toContain("boom");
        await vi.advanceTimersByTimeAsync(5000);
        expect(sample).toHaveBeenCalledTimes(2);
        expect(screen.queryByRole("alert")).toBeNull();
    });
});

/** A machine whose processes carry their kinds, as srv sends them. */
function kindSnapshot(): TowerSnapshot {
    const snap = snapshot(true);
    snap.tasks[0].processes[0] = { ...snap.tasks[0].processes[0], kind: "agents", provider: "claude" };
    snap.tasks[0].processes[1] = { ...snap.tasks[0].processes[1], kind: "builds" };
    snap.machine = { cpu: 3.3, mem: 8 * GB, processes: 7, mem_total: 16 * GB };
    snap.host!.processes = [
        {
            id: "10:1",
            pid: 10,
            name: "claude.exe",
            cpu: 0.5,
            mem: GB / 2,
            task: "block-a",
            kind: "agents",
            provider: "claude",
        },
        // 1.5 cores of 4: 38% of the machine, over a process's 25% warning.
        { id: "11:1", pid: 11, ppid: 10, name: "node.exe", cpu: 1.5, mem: GB / 2, task: "block-a", kind: "builds" },
        { id: "20:1", pid: 20, name: "chrome.exe", cpu: 0.4, mem: GB, kind: "browsers" },
        { id: "21:1", pid: 21, name: "chrome.exe", cpu: 0.2, mem: GB, kind: "browsers" },
        { id: "30:1", pid: 30, name: "svchost.exe", cpu: 0.05, mem: GB / 8, kind: "system" },
        { id: "31:1", pid: 31, name: "svchost.exe", cpu: 0.05, mem: GB / 8, kind: "system" },
        {
            id: "5:1",
            pid: 5,
            name: "agentmux-srv.exe",
            cpu: 0.1,
            mem: 2 * GB,
            task: "agentmux",
            kind: "agentmux",
            detail: "Server",
        },
    ];
    snap.host!.cpu = 2.8;
    snap.host!.mem = 5.25 * GB;
    snap.host!.total = 7;
    snap.host!.matched = 7;
    return snap;
}

describe("Tower: kinds of process", () => {
    beforeEach(() => {
        peersCmd.mockResolvedValue({ peers: [] });
        sample.mockImplementation(() => Promise.resolve(kindSnapshot()));
    });
    afterEach(() => {
        cleanup();
        vi.clearAllMocks();
        vi.useRealTimers();
        setMeta({});
        setVisibility("active");
    });

    const row = (name: string) => screen.getAllByText(name)[0].closest("tr")!;

    it("every process row starts with its kind's icon, in its colour; an agent's is its provider's logo", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        await screen.findByText("node.exe");
        expect(row("node.exe")).toHaveClass("tower-kind--builds");
        expect(within(row("node.exe")).getByRole("img", { name: "Builds & tools" })).toBeInTheDocument();
        const claudeIcon = within(row("claude.exe")).getByRole("img", { name: "Agents & AI" });
        expect(claudeIcon.querySelector(".fa-robot"), "the provider's logo, not the generic robot").toBeNull();
        expect(within(row("Server")).getByRole("img", { name: "AgentMux" })).toBeInTheDocument();
    });

    it("the system's processes start folded into one row, which opens to them", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        const system = await screen.findByTestId("tower-kind-system");
        expect(within(system).getByText("(2)")).toBeInTheDocument();
        expect(screen.queryByText("svchost.exe")).toBeNull();
        fireEvent.click(within(system).getByRole("button", { name: "Show System" }));
        expect(screen.getAllByText("svchost.exe")).toHaveLength(2);
    });

    it("the system folds in the list by app too, but not when it is the kind asked for", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "app" });
        renderTower();
        await screen.findByTestId("tower-kind-system");
        expect(screen.queryByText("svchost.exe")).toBeNull();
        cleanup();
        setMeta({ "tower:view": "processes", "tower:group": "none", "tower:kind": "system" });
        renderTower();
        await screen.findAllByText("svchost.exe");
        expect(screen.queryByTestId("tower-kind-system")).toBeNull();
    });

    it("a kind's chip shows only that kind, and Show all brings every kind back", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        const chip = await screen.findByTestId("tower-kind-chip-browsers");
        expect(chip).toHaveTextContent("Browsers");
        expect(chip).toHaveTextContent("2 ·");
        fireEvent.click(chip);
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:kind": "browsers" });
        await waitFor(() => expect(screen.queryByText("node.exe")).toBeNull());
        expect(screen.getAllByText("chrome.exe")).toHaveLength(2);
        expect(screen.getByTestId("tower-kind-chip-browsers")).toHaveAttribute("aria-pressed", "true");
        const only = screen.getByTestId("tower-kind-only");
        expect(only).toHaveTextContent("Only Browsers & web views");
        fireEvent.click(within(only).getByRole("button", { name: "Show all" }));
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:kind": null });
        await screen.findByText("node.exe");
    });

    it("groups by type: a header per kind present, the busiest open, the system last", async () => {
        setMeta({ "tower:view": "processes" });
        renderTower();
        await screen.findByTestId("tower-owner-block-a");
        fireEvent.click(screen.getByRole("radio", { name: "Type" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:group": "type" });
        await screen.findByTestId("tower-kind-builds");
        const headers = screen.getAllByTestId(/^tower-kind-(?!chip|only)/).map((r) => r.dataset.testid);
        expect(headers[headers.length - 1]).toBe("tower-kind-system");
        expect(headers).toEqual(
            expect.arrayContaining([
                "tower-kind-agents",
                "tower-kind-builds",
                "tower-kind-browsers",
                "tower-kind-agentmux",
            ])
        );
        // The two busiest kinds open: builds (1.5 cores) and browsers (0.6).
        expect(
            within(screen.getByTestId("tower-kind-builds")).getByRole("button", { name: "Hide Builds & tools" })
        ).toBeInTheDocument();
        expect(screen.getByText("node.exe")).toBeInTheDocument();
        expect(
            within(screen.getByTestId("tower-kind-system")).getByRole("button", { name: "Show System" })
        ).toBeInTheDocument();
    });

    it("a process of an agent's task carries the agent's colour in the flat list", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        await screen.findByText("node.exe");
        expect(row("node.exe")).toHaveClass("tower-process-row--owned");
        expect(row("node.exe").style.getPropertyValue("--tower-owner-color")).toBe("#ff8800");
        expect(row("chrome.exe")).not.toHaveClass("tower-process-row--owned");
    });

    it("a heavy figure is marked, and every figure has a bar for its share of the machine", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        await screen.findByText("node.exe");
        const [nodeMem, nodeCpu] = within(row("node.exe")).getAllByRole("cell").slice(1, 3);
        expect(nodeCpu).toHaveClass("tower-level--warn");
        expect(nodeMem).not.toHaveClass("tower-level--warn");
        // Half a GB of 16: about 3% of the machine's memory.
        expect((nodeMem.querySelector(".tower-load-bar") as HTMLElement).style.width).toBe("3.125%");
        const claudeCpu = within(row("claude.exe")).getAllByRole("cell")[2];
        expect(claudeCpu.className).not.toMatch(/tower-level--/);
    });

    it("the breakdown splits the machine's CPU and memory by kind", async () => {
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        const kinds = await screen.findByTestId("tower-kinds");
        // The memory bar comes first; CPU's is the one labelled CPU.
        const cpuBar = within(kinds).getByText("CPU").closest(".tower-kind-bar-row") as HTMLElement;
        const builds = cpuBar.querySelector(".tower-kind-segment.tower-kind--builds") as HTMLElement;
        expect(builds.title).toContain("Builds & tools · 1 process");
        // 1.5 cores of 4.
        expect(builds.style.width).toBe("37.5%");
        expect(within(kinds).getByText("5.3 GB of 16.0 GB")).toBeInTheDocument();
    });

    it("the agent's charts: CPU in its colour, Memory in the theme's memory colour", async () => {
        vi.useFakeTimers();
        let ts = 1_000_000;
        sample.mockImplementation(() => {
            const snap = kindSnapshot();
            snap.ts_ms = ts;
            ts += 2000;
            return Promise.resolve(snap);
        });
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        await vi.advanceTimersByTimeAsync(2000);
        const [mem, cpu] = screen.getAllByRole("img", { name: /over the last 10 minutes/ });
        expect((cpu.querySelector(".tower-chart-line") as SVGElement).style.stroke).toBe("rgb(255, 136, 0)");
        expect((mem.querySelector(".tower-chart-line") as SVGElement).style.stroke).toBe("var(--sysinfo-mem-color)");
    });
});

const MB = 1024 * 1024;

/** The Panes view's part of a sample: one window, its renderer, two tabs. */
function withPanes(snap: TowerSnapshot): TowerSnapshot {
    const renderer = { id: "60:1", pid: 60, name: "agentmux.exe", cpu: 0.2, mem: 900 * MB, detail: "Renderer" };
    const ghPage = { id: "61:1", pid: 61, name: "agentmux.exe", cpu: 0.1, mem: 150 * MB, detail: "Renderer" };
    const ghFrame = { id: "62:1", pid: 62, name: "agentmux.exe", cpu: 0, mem: 40 * MB, detail: "Renderer" };
    const shared = { id: "63:1", pid: 63, name: "agentmux.exe", cpu: 0, mem: 80 * MB, detail: "Renderer" };
    const pane = (block_id: string, view: string, title: string, state: string) => ({
        block_id,
        view,
        title,
        state,
        mem: 0,
        processes: [],
        shared: [],
    });
    return {
        ...snap,
        panes: {
            windows: [
                {
                    window_id: "w1",
                    name: "My Bizzy",
                    renderers: [renderer],
                    cpu: 0.2,
                    mem: 900 * MB,
                    tabs: [
                        {
                            tab_id: "t1",
                            name: "Tab 1",
                            shown: true,
                            panes: [
                                { ...pane("block-a", "agent", "AgentX", "shown"), task: "block-a", cpu: 2, mem: GB },
                                {
                                    ...pane("gh", "browser", "github.com", "shown"),
                                    cpu: 0.1,
                                    mem: 190 * MB,
                                    processes: [ghPage, ghFrame],
                                },
                                pane("sw", "swarm", "", "shown"),
                            ],
                        },
                        {
                            tab_id: "t2",
                            name: "Tab 2",
                            shown: false,
                            panes: [
                                { ...pane("news", "browser", "news.example", "hidden"), shared: [shared] },
                                pane("slept", "browser", "docs.agentmux.ai", "asleep"),
                            ],
                        },
                    ],
                },
            ],
            pool: [{ id: "70:1", pid: 70, name: "agentmux.exe", cpu: 0, mem: 60 * MB, detail: "Renderer" }],
            shared_mem: 80 * MB,
        },
    };
}

describe("Tower's Panes view", () => {
    beforeEach(() => {
        peersCmd.mockResolvedValue({ peers: [studio] });
        sample.mockImplementation((_client: unknown, req: { host: boolean; panes?: boolean }) =>
            Promise.resolve(req.panes ? withPanes(snapshot(req.host)) : snapshot(req.host))
        );
        setMeta({ "tower:view": "panes" });
    });
    afterEach(() => {
        cleanup();
        vi.clearAllMocks();
        setMeta({});
    });

    it("asks for panes and lists each window, its tabs and their panes", async () => {
        renderTower();
        await screen.findByTestId("tower-pane-window-w1");
        expect(sample).toHaveBeenCalledWith(expect.anything(), { host: false, panes: true });
        expect(screen.getByText("Window My Bizzy")).toBeInTheDocument();
        const order = screen.getAllByTestId(/^tower-pane-(tab|block|gh|sw|news|slept)/).map((r) => r.dataset.testid);
        expect(order).toEqual([
            "tower-pane-tab-t1",
            "tower-pane-block-a",
            "tower-pane-gh",
            "tower-pane-sw",
            "tower-pane-tab-t2",
            "tower-pane-news",
            "tower-pane-slept",
        ]);
    });

    it("measures agents and browser pages, and gives panes drawn in the window a dash", async () => {
        renderTower();
        const agent = await screen.findByTestId("tower-pane-block-a");
        expect(within(agent).getByText("1.0 GB")).toBeInTheDocument();
        expect(within(screen.getByTestId("tower-pane-gh")).getByText("190.0 MB")).toBeInTheDocument();
        const swarm = screen.getByTestId("tower-pane-sw");
        expect(within(swarm).getAllByText("—")).toHaveLength(2);
        expect(within(screen.getByTestId("tower-pane-window-w1")).getByText("900.0 MB")).toBeInTheDocument();
        expect(within(screen.getByTestId("tower-pane-slept")).getByText("no page")).toBeInTheDocument();
        expect(within(screen.getByTestId("tower-pane-slept")).getByText("asleep")).toBeInTheDocument();
    });

    it("opens a browser pane to its page's renderers, its frames' included", async () => {
        renderTower();
        const gh = await screen.findByTestId("tower-pane-gh");
        fireEvent.click(within(gh).getByRole("button", { name: "Show its processes" }));
        expect(screen.getByText("62")).toBeInTheDocument();
        expect(screen.getByText("61")).toBeInTheDocument();
    });

    it("says when a renderer is shared, and doesn't add it to the pane", async () => {
        renderTower();
        await screen.findByTestId("tower-pane-news");
        expect(screen.getByText(/Shares a renderer with another browser pane/)).toBeInTheDocument();
        expect(screen.getByText(/80.0 MB in renderers several browser panes share, counted once/)).toBeInTheDocument();
    });

    it("keeps each figure under its own header: the shared renderer's memory under Memory, a missing page's note too", async () => {
        renderTower();
        await screen.findByTestId("tower-pane-news");
        const headerIndex = (name: string) =>
            within(screen.getByRole("table", { name: "Panes" }))
                .getAllByRole("columnheader")
                .findIndex((h) => h.textContent?.trim() === name);
        const memory = headerIndex("Memory");
        const cpu = headerIndex("CPU");
        // The label spans Pane and State, so the figure that follows is Memory's and CPU's cell is empty.
        const shared = document.querySelector(".tower-panes-shared") as HTMLElement;
        const cells = [...shared.children] as HTMLElement[];
        expect(cells[0].getAttribute("colspan")).toBe("2");
        expect(cells.length).toBe(1 + (cpu - 1));
        expect(cells[memory - 1]).toHaveTextContent("MB");
        expect(cells[cpu - 1]).toBeEmptyDOMElement();
        // An asleep pane's "no page" is Memory's cell, the dash CPU's.
        const slept = [...screen.getByTestId("tower-pane-slept").children] as HTMLElement[];
        expect(slept[memory]).toHaveTextContent("no page");
        expect(slept[cpu]).toHaveTextContent("—");
    });

    it("goes to a pane from its row", async () => {
        renderTower();
        const gh = await screen.findByTestId("tower-pane-gh");
        fireEvent.click(within(gh).getByRole("button", { name: "Show this pane" }));
        expect(reveal).toHaveBeenCalledWith("gh", { source: "tower" });
    });

    it("isn't offered for another computer", async () => {
        setMeta({ "tower:view": "panes", "tower:connection": "peer:p1" });
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.queryByRole("tab", { name: /Panes/ })).toBeNull();
    });

    it("zooms the whole view with the pane's shared zoom", async () => {
        setMeta({ "term:zoom": 1.5 });
        renderTower();
        expect(screen.getByTestId("tower-view").style.zoom).toBe("1.5");
    });
});

/** SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md §3: memory is
 *  the first number, the default sort, and always on screen. */
describe("Tower: memory first", () => {
    beforeEach(() => {
        peersCmd.mockResolvedValue({ peers: [studio] });
        sample.mockImplementation((_client: unknown, req: { host: boolean; panes?: boolean }) =>
            Promise.resolve(req.panes ? withPanes(snapshot(req.host)) : snapshot(req.host))
        );
    });
    afterEach(() => {
        cleanup();
        vi.clearAllMocks();
        setMeta({});
    });

    const headers = (table: HTMLElement) =>
        within(table)
            .getAllByRole("columnheader")
            .map((h) => h.textContent?.trim());

    it("says AgentMux's total memory under the toolbar, whatever the view", async () => {
        renderTower();
        const headline = await screen.findByTestId("tower-headline");
        expect(headline).toHaveTextContent("AgentMux");
        // The agent's 1 GB and AgentMux's own 2 GB, over 3 processes.
        expect(within(headline).getByText("3.0 GB")).toBeInTheDocument();
        expect(headline).toHaveTextContent("3 processes");
        expect(headline.title).toContain("private working set");
        setMeta({ "tower:view": "processes" });
        expect(await screen.findByTestId("tower-headline")).toHaveTextContent("3.0 GB");
        setMeta({ "tower:view": "panes" });
        expect(await screen.findByTestId("tower-headline")).toHaveTextContent("3.0 GB");
    });

    it("doesn't total another computer's tasks, which are a capped list", async () => {
        sample.mockImplementation(() => Promise.resolve({ ...snapshot(), remote: true }));
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.queryByTestId("tower-headline")).toBeNull();
    });

    it("puts Memory before CPU in every table", async () => {
        setMeta({ "tower:view": "processes" });
        renderTower();
        await screen.findByRole("table", { name: "Processes" });
        // Grouped by agent, the last column counts a group's processes.
        expect(headers(screen.getByRole("table", { name: "Processes" }))).toEqual([
            "Process",
            "Memory",
            "CPU",
            "Count",
        ]);

        setMeta({ "tower:view": "agents" });
        const agentTable = await screen.findByRole("table", { name: "AgentX's processes" });
        expect(headers(agentTable)).toEqual(["Process", "Memory", "CPU", "PID"]);

        setMeta({ "tower:view": "panes" });
        const panes = await screen.findByRole("table", { name: "Panes" });
        expect(headers(panes)).toEqual(["Pane", "State", "Memory", "CPU"]);
    });

    it("sorts by memory, biggest first, until another sort is chosen", async () => {
        sample.mockImplementation(() => {
            const snap = snapshot(true);
            snap.host = {
                processes: [
                    { id: "1:1", pid: 1, name: "busy.exe", cpu: 2, mem: 10 * 1024 * 1024 },
                    { id: "2:1", pid: 2, name: "big.exe", cpu: 0.01, mem: 2 * GB },
                    { id: "3:1", pid: 3, name: "mid.exe", cpu: 0.5, mem: GB / 4 },
                ],
                unmeasured: 0,
                cpu: 2.51,
                mem: 2 * GB + GB / 4 + 10 * 1024 * 1024,
                total: 3,
                matched: 3,
            };
            return Promise.resolve(snap);
        });
        setMeta({ "tower:view": "processes", "tower:group": "none" });
        renderTower();
        await screen.findByText("big.exe");
        const order = () =>
            within(screen.getByRole("table", { name: "Processes" }))
                .getAllByRole("row")
                .slice(1)
                .map((r) => r.textContent ?? "");
        expect(order().map((t) => /(busy|big|mid)\.exe/.exec(t)?.[0])).toEqual(["big.exe", "mid.exe", "busy.exe"]);
        fireEvent.click(screen.getByRole("button", { name: /CPU/ }));
        await waitFor(() =>
            expect(order().map((t) => /(busy|big|mid)\.exe/.exec(t)?.[0])).toEqual(["busy.exe", "mid.exe", "big.exe"])
        );
    });

    it("shows memory first in the rail, then CPU, and keeps both in the row's text", async () => {
        renderTower();
        const row = await screen.findByTestId("tower-rail-block-a");
        const mem = row.querySelector(".tower-rail-mem") as HTMLElement;
        const cpu = row.querySelector(".tower-rail-cpu") as HTMLElement;
        expect(mem).toHaveTextContent("1.0 GB");
        expect(cpu).toHaveTextContent("CPU");
        // Memory is on the row's first line, ahead of CPU in the document.
        expect(mem.compareDocumentPosition(cpu) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        // Narrow panes hide CPU in the rail; the row's hover text still has both.
        expect(row.title).toBe("AgentX: 1.0 GB · CPU 50% · 2 processes");
    });

    it("heads the agent's detail with its memory, then its CPU", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        const totals = document.querySelector(".tower-detail-totals") as HTMLElement;
        expect(totals.textContent?.replace(/\s+/g, " ").trim()).toMatch(/^Memory 1\.0 GB · CPU .* · 2 processes$/);
    });

    it("keeps a memory history for the rail, apart from CPU", async () => {
        vi.useFakeTimers();
        let ts = 1_000_000;
        let mem = GB / 4;
        sample.mockImplementation(() => {
            const snap = snapshot();
            snap.ts_ms = ts;
            ts += 2000;
            snap.tasks[0].mem = mem;
            mem += GB / 16;
            return Promise.resolve(snap);
        });
        const model = new TowerViewModel({
            blockId: "tower-hist",
            meta: withAgentsView as unknown as PaneTabHostContext["meta"],
            setMeta: setMetaMock,
            isFocused: () => true,
            visibility,
        });
        render(() => <TowerView model={model} />);
        await vi.advanceTimersByTimeAsync(2000 * 6);
        const points = model.recentMem("block-a");
        expect(points.length).toBeGreaterThan(3);
        expect(points[points.length - 1]).toBeGreaterThan(points[0]);
    });
});

const MB_ = 1024 * 1024;

/** The Memory view's part of a sample: four parts, an agent with its roles, a
 *  stray Claude, and another instance beside the total. */
function withMemory(snap: TowerSnapshot, opts: { hour?: number; growth?: boolean } = {}): TowerSnapshot {
    const row = (key: string, label: string, mem: number, extra: object = {}) => ({
        key,
        label,
        mem: mem * MB_,
        processes: 1,
        children: [],
        ...extra,
    });
    snap.memory = {
        total: 1000 * MB_,
        processes: 14,
        unreadable: 2,
        metric: "private working set",
        growth: [],
        other_instances: 512 * MB_,
        parts: [
            {
                part: "core",
                mem: 300 * MB_,
                processes: 4,
                rows: [row("core:gpu-process", "GPU process", 200), row("core:server-srv", "Server (srv)", 100)],
            },
            {
                part: "panes",
                mem: 100 * MB_,
                processes: 2,
                rows: [row("terminal:t1", "Terminal", 100, { task: "t1", kind: "shells", processes: 2 })],
            },
            {
                part: "agents",
                mem: 550 * MB_,
                processes: 6,
                rows: [
                    row("agent:block-a", "AgentX", 550, {
                        task: "block-a",
                        kind: "agents",
                        processes: 6,
                        children: [
                            { key: "agent:block-a:cli", label: "CLI", mem: 400 * MB_, processes: 2 },
                            { key: "agent:block-a:tools", label: "Tools it started", mem: 150 * MB_, processes: 4 },
                        ],
                    }),
                ],
            },
            {
                part: "not_placed",
                mem: 50 * MB_,
                processes: 2,
                rows: [row("np:claude-exe", "claude.exe", 50, { processes: 2, kind: "agents" })],
            },
        ],
    };
    if (opts.growth) {
        snap.memory.growth = [{ key: "agent:block-a", from: 400 * MB_, to: 550 * MB_, minutes: 20 }];
    }
    if (opts.hour) snap.memory.history = hourOf(opts.hour);
    return snap;
}

/** The srv ledger's hour: `n` points ten seconds apart. The total climbs
 *  800 to 1000 MB, Agents 400 to 550; Core is flat; Not placed has no line. */
function hourOf(n: number) {
    const ramp = (a: number, b: number) =>
        Array.from({ length: n }, (_, i) => Math.round(a + ((b - a) * i) / (n - 1)) * MB_);
    const start_ms = 1_700_000_000_000;
    return {
        step_ms: 10_000,
        since_ms: start_ms,
        series: [
            { key: "total", start_ms, points: ramp(800, 1000) },
            { key: "part:core", start_ms, points: ramp(300, 300) },
            { key: "part:agents", start_ms, points: ramp(400, 550) },
            { key: "agent:block-a", start_ms, points: ramp(400, 550) },
            { key: "core:gpu-process", start_ms, points: ramp(200, 200) },
        ],
    };
}

/** What the mocked srv sends on the next Memory sample. */
let memoryExtras: { growth?: boolean; hourPoints?: number } = {};

describe("Tower: the Memory view", () => {
    beforeEach(() => {
        peersCmd.mockResolvedValue({ peers: [studio] });
        memoryExtras = {};
        sample.mockImplementation(
            (_client: unknown, req: { host: boolean; memory?: boolean; memory_history?: boolean }) =>
                Promise.resolve(
                    req.memory
                        ? withMemory(snapshot(req.host), {
                              growth: memoryExtras.growth,
                              hour: req.memory_history ? memoryExtras.hourPoints : undefined,
                          })
                        : snapshot(req.host)
                )
        );
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        vi.clearAllMocks();
        setMeta({});
    });

    it("is where Tower opens, and asks for the memory", async () => {
        renderTower({ rawMeta: true });
        await screen.findByTestId("tower-memory");
        expect(sample).toHaveBeenCalledWith(expect.anything(), { host: false, memory: true, memory_history: true });
        expect(screen.getByRole("tab", { name: /Memory/ })).toHaveAttribute("aria-selected", "true");
        // The Agents view is one click away, and is remembered.
        fireEvent.click(screen.getByRole("tab", { name: /Agents/ }));
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:view": "agents" });
    });

    it("opens a pane that chose Agents on Agents", async () => {
        setMeta({ "tower:view": "agents" });
        renderTower({ rawMeta: true });
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.queryByTestId("tower-memory")).toBeNull();
    });

    it("draws one bar of the four parts, each its share of the total", async () => {
        renderTower({ rawMeta: true });
        await screen.findByTestId("tower-memory");
        const widths = ["core", "panes", "agents", "not_placed"].map(
            (p) => (screen.getByTestId(`tower-memory-segment-${p}`) as HTMLElement).style.width
        );
        expect(widths.map((w) => Math.round(parseFloat(w)))).toEqual([30, 10, 55, 5]);
        expect(
            screen.getByRole("group", { name: /^AgentMux's memory by part: Core: 300.0 MB, 30%; Panes/ })
        ).toBeInTheDocument();
    });

    it("lists the parts in order with their memory and share, the busiest open", async () => {
        renderTower({ rawMeta: true });
        const agents = await screen.findByTestId("tower-memory-part-agents");
        const order = screen.getAllByTestId(/^tower-memory-part-/).map((r) => r.dataset.testid);
        expect(order).toEqual([
            "tower-memory-part-core",
            "tower-memory-part-panes",
            "tower-memory-part-agents",
            "tower-memory-part-not_placed",
        ]);
        expect(within(agents).getByText("550.0 MB")).toBeInTheDocument();
        expect(within(agents).getByText("55%")).toBeInTheDocument();
        // Agents is the biggest: its agent is shown, the smaller parts are shut.
        expect(screen.getByTestId("tower-memory-row-agent:block-a")).toBeInTheDocument();
        expect(screen.queryByTestId("tower-memory-row-core:gpu-process")).toBeNull();
    });

    it("opens a part to its rows, and an agent to its CLI, helpers and tools", async () => {
        renderTower({ rawMeta: true });
        const core = await screen.findByTestId("tower-memory-part-core");
        fireEvent.click(within(core).getByRole("button", { name: /Show what's in Core/ }));
        expect(
            within(screen.getByTestId("tower-memory-row-core:gpu-process")).getByText("200.0 MB")
        ).toBeInTheDocument();
        const agent = screen.getByTestId("tower-memory-row-agent:block-a");
        fireEvent.click(within(agent).getByRole("button", { name: /Show AgentX's parts/ }));
        expect(
            within(screen.getByTestId("tower-memory-child-agent:block-a:cli")).getByText("400.0 MB")
        ).toBeInTheDocument();
        expect(screen.getByTestId("tower-memory-child-agent:block-a:tools")).toHaveTextContent("Tools it started");
    });

    it("says what Not placed is, by name, and never puts it in Core", async () => {
        renderTower({ rawMeta: true });
        const np = await screen.findByTestId("tower-memory-part-not_placed");
        fireEvent.click(within(np).getByRole("button", { name: /Show what's in Not placed/ }));
        expect(screen.getByText(/AgentMux started these and no pane claims them/)).toBeInTheDocument();
        const claude = screen.getByTestId("tower-memory-row-np:claude-exe");
        expect(claude).toHaveTextContent("claude.exe");
        expect(claude).toHaveTextContent("×2");
        const core = screen.getByTestId("tower-memory-part-core");
        fireEvent.click(within(core).getByRole("button", { name: /Show what's in Core/ }));
        expect(within(screen.getByTestId("tower-memory")).getAllByText(/claude/)).toHaveLength(1);
    });

    it("goes from an agent's row to that agent in the Agents view", async () => {
        renderTower({ rawMeta: true });
        const agent = await screen.findByTestId("tower-memory-row-agent:block-a");
        fireEvent.click(within(agent).getByRole("button", { name: /in the Agents view/ }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:agent": "block-a" });
        expect(setMetaMock).toHaveBeenLastCalledWith({ "tower:view": "agents" });
    });

    it("says what the number is, what could not be read, and another instance beside it", async () => {
        renderTower({ rawMeta: true });
        const note = await screen.findByTestId("tower-memory-note");
        expect(note).toHaveTextContent("14 processes, measured as private working set");
        expect(note).toHaveTextContent(
            "Shared libraries and shared memory aren't counted twice, and video memory isn't counted"
        );
        expect(note).toHaveTextContent("2 processes couldn't be read");
        expect(screen.getByTestId("tower-memory-others")).toHaveTextContent("512.0 MB, not counted here");
    });

    it("isn't offered for another computer", async () => {
        setMeta({ "tower:connection": "peer:p1" });
        renderTower({ rawMeta: true });
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.queryByRole("tab", { name: /Memory/ })).toBeNull();
        expect(screen.queryByTestId("tower-memory")).toBeNull();
    });
});

describe("Tower: which line the Memory chart draws", () => {
    const memory = withMemory(snapshot()).memory!;
    const series = (key: string) => ({ key, start_ms: 0, points: [1, 2] });
    const trend = {
        byKey: new Map(["total", "part:agents", "agent:block-a", "agent:gone"].map((k) => [k, series(k)])),
        end: 10_000,
        spanMs: 3_600_000,
    };

    it("is the picked part or row while it is on screen and the ledger has it", () => {
        expect(chartKey("part:agents", memory, trend)).toBe("part:agents");
        expect(chartKey("agent:block-a", memory, trend)).toBe("agent:block-a");
    });

    it("is the total for a row that has left the table, though the ledger still holds its line", () => {
        expect(chartKey("agent:gone", memory, trend)).toBe("total");
    });

    it("is the total for a line the ledger has none of, or before the hour arrives", () => {
        expect(chartKey("part:not_placed", memory, trend)).toBe("total");
        expect(chartKey("agent:block-a", memory, null)).toBe("total");
    });
});

describe("Tower: the Memory view's hour and what grows", () => {
    beforeEach(() => {
        peersCmd.mockResolvedValue({ peers: [studio] });
        memoryExtras = {};
        sample.mockImplementation(
            (_client: unknown, req: { host: boolean; memory?: boolean; memory_history?: boolean }) =>
                Promise.resolve(
                    req.memory
                        ? withMemory(snapshot(req.host), {
                              growth: memoryExtras.growth,
                              hour: req.memory_history ? memoryExtras.hourPoints : undefined,
                          })
                        : snapshot(req.host)
                )
        );
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        vi.clearAllMocks();
        setMeta({});
    });

    it("asks for the hour with the first sample and then every ten seconds, not every sample", async () => {
        vi.useFakeTimers();
        memoryExtras = { hourPoints: 360 };
        const model = new TowerViewModel({
            blockId: "tower-hour",
            meta: meta as unknown as PaneTabHostContext["meta"],
            setMeta: setMetaMock,
            isFocused: () => true,
            visibility,
        });
        render(() => <TowerView model={model} />);
        const withHour = () =>
            sample.mock.calls.filter((c) => (c[1] as { memory_history?: boolean }).memory_history).length;
        await vi.advanceTimersByTimeAsync(100);
        expect(withHour()).toBe(1);
        await vi.advanceTimersByTimeAsync(6000);
        expect(sample.mock.calls.length).toBeGreaterThan(2);
        expect(withHour()).toBe(1);
        await vi.advanceTimersByTimeAsync(5000);
        expect(withHour()).toBe(2);
    });

    it("draws the hour as a line per part and row, none where the ledger has no line", async () => {
        memoryExtras = { hourPoints: 360 };
        renderTower({ rawMeta: true });
        const line = async (testid: string) => {
            const row = await screen.findByTestId(testid);
            return row.querySelector(".tower-memory-spark polyline")?.getAttribute("points") ?? "";
        };
        await waitFor(async () => expect(await line("tower-memory-part-agents")).not.toBe(""));
        expect((await line("tower-memory-row-agent:block-a")).split(" ").length).toBe(360);
        expect(await line("tower-memory-part-not_placed")).toBe("");
        // Before the hour arrives, no line and no chart: the table still reads.
    });

    it("charts the total, then whichever part or row is chosen, over 1 hour or 10 minutes", async () => {
        memoryExtras = { hourPoints: 360 };
        renderTower({ rawMeta: true });
        const label = async () => (await screen.findByTestId("tower-memory-chart-label")).textContent;
        await waitFor(async () => expect(await label()).toBe("AgentMux"));
        const chart = screen.getByTestId("tower-memory-chart");
        expect(chart).toHaveTextContent("1 hour ago");
        fireEvent.click(screen.getByTestId("tower-memory-part-agents"));
        expect(await label()).toBe("Agents");
        fireEvent.click(screen.getByTestId("tower-memory-row-agent:block-a"));
        expect(await label()).toBe("AgentX");
        fireEvent.click(within(chart).getByRole("radio", { name: "10 min" }));
        expect(chart).toHaveTextContent("10 min ago");
        // A row with no line charts the total.
        fireEvent.click(screen.getByTestId("tower-memory-part-not_placed"));
        expect(await label()).toBe("AgentMux");
    });

    it("says what changed over the hour, and since when before an hour exists", async () => {
        memoryExtras = { hourPoints: 360 };
        renderTower({ rawMeta: true });
        const change = await screen.findByTestId("tower-headline-change");
        expect(change).toHaveTextContent("+200.0 MB in the last hour");
        cleanup();
        memoryExtras = { hourPoints: 90 };
        renderTower({ rawMeta: true });
        const since = await screen.findByTestId("tower-headline-change");
        expect(since).toHaveTextContent(/^\+200\.0 MB since \d/);
        expect(await screen.findByTestId("tower-memory-chart")).toHaveTextContent(/since \d/);
    });

    it("flags what grows unusually on the row, on its part and on the headline", async () => {
        memoryExtras = { hourPoints: 360, growth: true };
        renderTower({ rawMeta: true });
        const row = await screen.findByTestId("tower-memory-grow-agent:block-a");
        expect(row).toHaveTextContent("grew 150.0 MB in 20 min");
        // The part carries it too, with the row's name, visible with the table shut.
        expect(screen.getByTestId("tower-memory-grow-part-agents")).toHaveTextContent("AgentX grew 150.0 MB in 20 min");
        expect(screen.queryByTestId("tower-memory-grow-part-core")).toBeNull();
        const dot = screen.getByTestId("tower-headline-grow");
        expect(dot).toHaveAttribute("title", "AgentX grew 150.0 MB in 20 min");
        expect(dot).toHaveAccessibleName(/Grows unusually: AgentX grew 150.0 MB in 20 min/);
    });

    it("shows no warning when nothing grows", async () => {
        memoryExtras = { hourPoints: 360 };
        renderTower({ rawMeta: true });
        await screen.findByTestId("tower-memory-chart");
        expect(screen.queryByTestId("tower-headline-grow")).toBeNull();
        expect(document.querySelector(".tower-memory-grow")).toBeNull();
    });

    it("keeps the hour out of the other views", async () => {
        memoryExtras = { hourPoints: 360, growth: true };
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        expect(screen.queryByTestId("tower-headline-change")).toBeNull();
        expect(screen.queryByTestId("tower-headline-grow")).toBeNull();
        expect(sample).toHaveBeenCalledWith(expect.anything(), { host: false });
    });
});
