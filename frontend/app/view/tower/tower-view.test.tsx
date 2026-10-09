// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const sample = vi.fn();
const commandLine = vi.fn();
const reveal = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        TowerSampleCommand: (...args: unknown[]) => sample(...args),
        TowerCommandLineCommand: (...args: unknown[]) => commandLine(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: (...args: unknown[]) => reveal(...args) }));

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

    it("expands a task into its processes and fetches a command line on demand", async () => {
        commandLine.mockResolvedValue({ command_line: "node build.js --watch" });
        renderTower();
        const agent = await screen.findByTestId("tower-task-block-a");
        expect(screen.queryByText("node.exe")).toBeNull();
        fireEvent.click(within(agent).getByRole("button", { name: "Show processes" }));
        const node = screen.getByText("node.exe").closest("tr")!;
        fireEvent.click(within(node).getByRole("button", { name: "Show command line" }));
        expect(await within(node).findByText("node build.js --watch")).toBeInTheDocument();
        expect(commandLine).toHaveBeenCalledWith(expect.anything(), { id: "11:1" });
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
