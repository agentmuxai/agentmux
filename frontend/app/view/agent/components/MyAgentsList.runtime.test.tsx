// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// My Agents shows each agent's runtime state on its row, and clicking one that runs with no pane
// attaches to it (store/agent-runtime.ts).

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MyAgentsList } from "./MyAgentsList";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListRecentSessionsCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/element/ProviderLogo", () => ({ ProviderLogo: () => null }));
const requestShutdown = vi.hoisted(() => vi.fn());
vi.mock("../shutdown/shut-down-agent", () => ({ requestShutdown }));

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
});
afterEach(cleanup);

const makeRow = (overrides: Partial<RecentSessionRow> = {}): RecentSessionRow => ({
    instance_id: "inst-1",
    instance_name: "Maks",
    definition_id: "def-1",
    definition_name: "Claude Code",
    provider: "claude",
    working_directory: "/tmp/maks",
    identity_id: "id-work",
    identity_name: "Work",
    memory_id: "mem-notes",
    memory_name: "Notes",
    block_id_hint: "blk-1",
    preview: "fix the hover delay",
    node_count: 1,
    last_active_at: Date.now() - 60_000,
    has_snapshot: true,
    agent_created_at: Date.now() - 7_776_000_000,
    started_at: Date.now() - 3_600_000,
    model_vendor_base_url: "",
    session_id: "",
    snapshot_check_failed: false,
    agent_type: "",
    ...overrides,
});

const runtime = (state: AgentRuntime["state"]): AgentRuntime => ({
    agent_id: "def-1",
    state,
    desired: "running",
    name: "Maks",
    provider: "claude",
    icon: "",
});

const load = (rows: RecentSessionRow[]) =>
    vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({ rows, degraded: [] });

describe("MyAgentsList Shut down", () => {
    const openMenu = async () => userEvent.click(await screen.findByTestId("agent-my-agents-menu-toggle"));

    it("offers Shut down on a running agent, pane or no pane, and shuts down the block it runs in", async () => {
        load([makeRow()]);
        render(() => <MyAgentsList onReattach={() => {}} runtimeOf={() => ({ ...runtime("idle"), block_id: "blk-9" })} />);
        await openMenu();
        await userEvent.click(await screen.findByTestId("agent-my-agents-shutdown"));
        expect(requestShutdown).toHaveBeenCalledWith("blk-9", "Maks");
    });

    it("offers nothing for a stopped agent or one with no block", async () => {
        load([makeRow()]);
        const { unmount } = render(() => (
            <MyAgentsList onReattach={() => {}} runtimeOf={() => ({ ...runtime("stopped"), block_id: "blk-9" })} />
        ));
        await openMenu();
        expect(screen.queryByTestId("agent-my-agents-shutdown")).toBeNull();
        unmount();
        load([makeRow()]);
        render(() => <MyAgentsList onReattach={() => {}} runtimeOf={() => runtime("idle")} />);
        await openMenu();
        expect(screen.queryByTestId("agent-my-agents-shutdown")).toBeNull();
    });
});

describe("MyAgentsList runtime state", () => {
    it("puts the agent's state on its row, and none on a row the store doesn't know", async () => {
        load([makeRow(), makeRow({ definition_id: "def-2", instance_name: "Other", instance_id: "inst-2" })]);
        const { container } = render(() => (
            <MyAgentsList
                onReattach={() => {}}
                runtimeOf={(id) => (id === "def-1" ? runtime("waiting_on_user") : undefined)}
            />
        ));
        await screen.findAllByTestId("agent-my-agents-entry");
        const chips = container.querySelectorAll(".agent-runtime-chip");
        expect(chips.length).toBe(1);
        expect(chips[0].textContent).toBe("waiting on you");
        expect(chips[0].closest("li")!.getAttribute("data-definition-id")).toBe("def-1");
    });

    it("asks before attaching to an agent that runs with no pane: Open it attaches, instead of starting another", async () => {
        load([makeRow()]);
        const onReattach = vi.fn();
        const onAttach = vi.fn();
        render(() => (
            <MyAgentsList onReattach={onReattach} onAttach={onAttach} runtimeOf={() => runtime("working")} />
        ));
        await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
        expect(onAttach).not.toHaveBeenCalled();
        expect(screen.getByTestId("agent-row-panel").textContent).toContain("is running with no pane");
        await userEvent.click(screen.getByTestId("agent-fork-attach"));
        expect(onAttach).toHaveBeenCalledTimes(1);
        expect(onAttach.mock.calls[0][0].definition_id).toBe("def-1");
        expect(onReattach).not.toHaveBeenCalled();
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });

    it("Fork on that prompt starts the named-copy flow, not an attach", async () => {
        load([makeRow()]);
        const onAttach = vi.fn();
        render(() => <MyAgentsList onReattach={() => {}} onAttach={onAttach} runtimeOf={() => runtime("idle")} />);
        await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
        await userEvent.click(screen.getByTestId("agent-fork-fork"));
        expect(onAttach).not.toHaveBeenCalled();
        expect(screen.queryByTestId("agent-fork-attach")).toBeNull();
        expect(await screen.findByTestId("agent-fork-name-input")).toBeTruthy();
    });

    it("gives an agent that runs with no pane its own look; an open one and a stopped one don't have it", async () => {
        load([
            makeRow(),
            makeRow({ definition_id: "def-2", instance_name: "Shown", instance_id: "inst-2" }),
            makeRow({ definition_id: "def-3", instance_name: "Off", instance_id: "inst-3" }),
        ]);
        const states: Record<string, AgentRuntime["state"]> = { "def-1": "idle", "def-2": "working", "def-3": "stopped" };
        const { container } = render(() => (
            <MyAgentsList
                onReattach={() => {}}
                onAttach={() => {}}
                runtimeOf={(id) => runtime(states[id])}
                openDefinitions={() => new Map([["def-2", "blk-9"]])}
            />
        ));
        await screen.findAllByTestId("agent-my-agents-entry");
        const row = (id: string) => container.querySelector(`li[data-definition-id="${id}"]`)!;
        expect(row("def-1").classList.contains("agent-recent-sessions-row--no-pane")).toBe(true);
        expect(row("def-1").querySelector(".agent-no-pane-tag")?.textContent).toBe("no pane");
        expect(row("def-2").classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
        expect(row("def-2").classList.contains("agent-recent-sessions-row--active")).toBe(true);
        expect(row("def-2").querySelector(".agent-no-pane-tag")).toBeNull();
        expect(row("def-3").classList.contains("agent-recent-sessions-row--no-pane")).toBe(false);
        expect(row("def-3").querySelector(".agent-no-pane-tag")).toBeNull();
    });

    it("starts a stopped agent from the panel's Open, the way it always did", async () => {
        load([makeRow()]);
        const onReattach = vi.fn();
        const onAttach = vi.fn();
        render(() => (
            <MyAgentsList onReattach={onReattach} onAttach={onAttach} runtimeOf={() => runtime("stopped")} />
        ));
        await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
        // The tile opens the panel; the start is its Open.
        expect(onReattach).not.toHaveBeenCalled();
        await userEvent.click(await screen.findByTestId("agent-my-agents-open"));
        expect(onReattach).toHaveBeenCalledTimes(1);
        expect(onAttach).not.toHaveBeenCalled();
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });

    it("keeps the already-open prompt for an agent whose pane is shown", async () => {
        load([makeRow()]);
        const onAttach = vi.fn();
        render(() => (
            <MyAgentsList
                onReattach={() => {}}
                onAttach={onAttach}
                runtimeOf={() => runtime("idle")}
                openDefinitions={() => new Map([["def-1", "blk-9"]])}
            />
        ));
        await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
        expect(onAttach).not.toHaveBeenCalled();
        expect(screen.getByTestId("agent-fork-switch")).toBeInTheDocument();
        expect(screen.queryByTestId("agent-fork-attach")).toBeNull();
    });
});
