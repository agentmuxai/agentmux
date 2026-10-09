// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const sample = vi.fn();
const reveal = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        TowerSampleCommand: (...args: unknown[]) => sample(...args),
    },
}));
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
                processes: [{ id: "5:1", pid: 5, name: "agentmux-srv.exe", cpu: 0.1, mem: 2 * GB, role: "main" }],
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
            ["build-box", false],
            ["mac-mini", false],
            ["pi", true],
            ["wsl://Ubuntu", false],
            ["win-server", true],
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
