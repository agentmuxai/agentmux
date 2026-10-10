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
                    { id: "5:1", pid: 5, name: "agentmux-srv.exe", cpu: 0.1, mem: 2 * GB, role: "main", detail: "Server" },
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

function renderTower() {
    return render(() => {
        const model = new TowerViewModel({
            blockId: "tower-1",
            meta: meta as unknown as PaneTabHostContext["meta"],
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

    it("shows a share of one core when asked", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        fireEvent.click(screen.getByRole("radio", { name: "% of a core" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:cpu": "core" });
        expect(within(screen.getByTestId("tower-rail-block-a")).getByText("200%")).toBeInTheDocument();
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
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:view": "processes" });
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
        expect(within(srv).getByText("agentmux-srv.exe")).toHaveClass("tower-muted");
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

    it("reveals an agent's pane, and offers no pane for AgentMux itself", async () => {
        renderTower();
        await screen.findByTestId("tower-rail-block-a");
        fireEvent.click(screen.getByRole("button", { name: "Show this pane" }));
        expect(reveal).toHaveBeenCalledWith("block-a");
        fireEvent.click(screen.getByTestId("tower-rail-agentmux"));
        await waitFor(() => expect(screen.queryByRole("button", { name: "Show this pane" })).toBeNull());
    });

    it("the Processes view lists every process, filters, and says what it can't measure", async () => {
        // "host" is what the view was saved as before it was renamed.
        setMeta({ "tower:view": "host" });
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
        setMeta({ "tower:view": "processes" });
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

        fireEvent.click(screen.getByRole("button", { name: "Group by app" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:group": "off" });
        await waitFor(() => expect(screen.queryByTestId("tower-app-chrome.exe")).toBeNull());
        expect(screen.getAllByText("chrome.exe")).toHaveLength(2);
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
