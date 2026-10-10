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
                    { id: "10:1", pid: 10, name: "claude.exe", cpu: 0.5, mem: GB / 2, role: "main" },
                    { id: "11:1", pid: 11, name: "node.exe", cpu: 1.5, mem: GB / 2, role: "started" },
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

    it("lists tasks by CPU as a share of the machine", async () => {
        renderTower();
        const agent = await screen.findByTestId("tower-task-block-a");
        // 2 cores busy of 4: 50%.
        expect(within(agent).getByText("50%")).toBeInTheDocument();
        expect(within(agent).getByText("1.0 GB")).toBeInTheDocument();
        expect(within(agent).getByText("agent")).toBeInTheDocument();
        const rows = screen.getAllByTestId(/tower-task-/).map((r) => r.dataset.testid);
        expect(rows).toEqual(["tower-task-block-a", "tower-task-agentmux"]);
        expect(screen.getByText(/2 tasks · 3 processes/)).toBeInTheDocument();
        expect(sample).toHaveBeenCalledWith(expect.anything(), { host: false });
    });

    it("shows a share of one core when asked", async () => {
        renderTower();
        await screen.findByTestId("tower-task-block-a");
        fireEvent.click(screen.getByRole("radio", { name: "% of a core" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "tower:cpu": "core" });
        expect(within(screen.getByTestId("tower-task-block-a")).getByText("200%")).toBeInTheDocument();
    });

    it("expands a task into its processes", async () => {
        renderTower();
        const agent = await screen.findByTestId("tower-task-block-a");
        expect(screen.queryByText("node.exe")).toBeNull();
        fireEvent.click(within(agent).getByRole("button", { name: "Show processes" }));
        const node = screen.getByText("node.exe").closest("tr")!;
        // 1.5 cores of 4.
        expect(within(node).getByText("38%")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /command line/i })).toBeNull();
    });

    it("says what each of AgentMux's own processes is, with its executable alongside", async () => {
        renderTower();
        const agentmux = await screen.findByTestId("tower-task-agentmux");
        fireEvent.click(within(agentmux).getByRole("button", { name: "Show processes" }));
        const srv = screen.getByText("Server").closest("tr")!;
        expect(within(srv).getByText("agentmux-srv.exe")).toHaveClass("tower-muted");
    });

    it("keeps each row, and updates it, across a refresh", async () => {
        vi.useFakeTimers();
        renderTower();
        await vi.advanceTimersByTimeAsync(0);
        const agent = screen.getByTestId("tower-task-block-a");
        fireEvent.click(within(agent).getByRole("button", { name: "Show processes" }));
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
        const agent = await screen.findByTestId("tower-task-block-a");
        fireEvent.click(within(agent).getByRole("button", { name: "Show this pane" }));
        expect(reveal).toHaveBeenCalledWith("block-a");
        expect(
            within(screen.getByTestId("tower-task-agentmux")).queryByRole("button", { name: "Show this pane" })
        ).toBeNull();
    });

    it("the Host view lists every process, filters, and says what it can't measure", async () => {
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
        await screen.findByTestId("tower-task-block-a");
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
        expect(screen.queryByRole("tab", { name: "Tasks" })).toBeNull();
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
        await screen.findByTestId("tower-task-block-a");
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
        await screen.findByTestId("tower-task-block-a");
        fireEvent.change(screen.getByRole("combobox", { name: "Machine" }), { target: { value: "__pair__" } });
        fireEvent.input(screen.getByLabelText("Pairing link"), { target: { value: "agentmux://pair?v=1" } });
        fireEvent.click(screen.getByRole("button", { name: "Pair" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("The code was refused");
        expect(screen.getByTestId("tower-pair")).toBeInTheDocument();
    });

    it("a paired AgentMux computer shows its tasks, but none of its panes can be revealed here", async () => {
        sample.mockImplementation(() => Promise.resolve({ ...snapshot(true), remote: true, hostname: "studio" }));
        setMeta({ "tower:connection": "peer:p1" });
        renderTower();
        const agent = await screen.findByTestId("tower-task-block-a");
        expect(screen.getByRole("tab", { name: "Tasks" })).toBeInTheDocument();
        expect(within(agent).queryByRole("button", { name: "Show this pane" })).toBeNull();
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

    it("the Host view groups processes of one app, and can list them flat", async () => {
        const many = snapshot(true);
        many.host!.processes = [
            { id: "20:1", pid: 20, name: "chrome.exe", cpu: 0.5, mem: GB },
            { id: "21:1", pid: 21, name: "chrome.exe", cpu: 0.25, mem: GB },
            { id: "4:1", pid: 4, name: "System", cpu: 0.01, mem: 1024 },
        ];
        sample.mockResolvedValue(many);
        setMeta({ "tower:view": "host" });
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
