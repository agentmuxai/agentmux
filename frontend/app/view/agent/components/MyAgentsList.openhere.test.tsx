// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Open here" in a My Agents row's panel: an agent that is open in another pane and has no turn running
// can be moved into this pane. A working agent keeps Switch to existing and Fork… only.

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { takeOpenHere } from "../failure/open-here";
import { MyAgentsList } from "./MyAgentsList";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListRecentSessionsCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/element/ProviderLogo", () => ({ ProviderLogo: () => null }));
vi.mock("../failure/open-here", async (orig) => ({ ...(await orig<typeof import("../failure/open-here")>()), closeHolderPane: vi.fn() }));

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
});
afterEach(cleanup);

const row = (): RecentSessionRow => ({
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
});

const runtime = (state: AgentRuntime["state"]): AgentRuntime =>
    ({ agent_id: "def-1", state, desired: "running", name: "Maks", provider: "claude", icon: "", runtime: "host" }) as AgentRuntime;

async function mount(opts: { state?: AgentRuntime["state"]; open?: boolean; goneAtClick?: boolean; onReattach?: () => unknown }) {
    vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({ rows: [row()], degraded: [] });
    const open = opts.open ?? true;
    // The row's own read and the click's read: what is open now.
    const there = new Map([["def-1", "blk-other"]]);
    const read = vi.fn(async () => (open ? there : new Map<string, string>()));
    // The first read is the one the panel makes as it opens; the click reads again.
    if (opts.goneAtClick) read.mockResolvedValueOnce(there).mockResolvedValue(new Map());
    render(() => (
        <MyAgentsList
            paneBlockId="blk-picker"
            onReattach={() => opts.onReattach?.()}
            openDefinitions={() => (open ? new Map([["def-1", "blk-other"]]) : new Map())}
            resolveOpenDefinitions={read}
            runtimeOf={() => (opts.state ? runtime(opts.state) : undefined)}
        />
    ));
    await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
    await screen.findByTestId("agent-row-actions");
}

describe("Open here", () => {
    it("is offered, beside Switch to existing and Fork…, for an idle agent open in another pane", async () => {
        await mount({ state: "idle" });
        expect(screen.getByTestId("agent-open-here").textContent).toBe("Open here");
        expect(screen.getByTestId("agent-fork-switch")).toBeInTheDocument();
        expect(screen.getByTestId("agent-fork-open-new")).toBeInTheDocument();
    });

    it("is not offered for a working agent, one waiting on the person, or one with no known state", async () => {
        for (const state of ["working", "waiting_on_user", undefined] as const) {
            await mount({ state });
            expect(screen.queryByTestId("agent-open-here"), String(state)).toBeNull();
            expect(screen.getByTestId("agent-fork-switch")).toBeInTheDocument();
            cleanup();
        }
    });

    it("opens the agent in this pane, marked so the refusal is answered with Move here, and closes the panel", async () => {
        const onReattach = vi.fn();
        await mount({ state: "idle", onReattach });
        await userEvent.click(screen.getByTestId("agent-open-here"));
        expect(onReattach).toHaveBeenCalledTimes(1);
        expect(takeOpenHere("blk-picker")).toBe(true);
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });

    it("leaves no mark when the open fails", async () => {
        const onReattach = vi.fn().mockRejectedValue(new Error("no"));
        await mount({ state: "idle", onReattach });
        await userEvent.click(screen.getByTestId("agent-open-here"));
        expect(takeOpenHere("blk-picker")).toBe(false);
    });

    it("is a plain open, with no mark, when the agent has closed elsewhere since the panel opened", async () => {
        const onReattach = vi.fn();
        await mount({ state: "idle", goneAtClick: true, onReattach });
        await userEvent.click(screen.getByTestId("agent-open-here"));
        expect(onReattach).toHaveBeenCalledTimes(1);
        expect(takeOpenHere("blk-picker")).toBe(false);
    });
});
