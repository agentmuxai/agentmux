// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const open = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: open }));
vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rows: () => [] }) }));

import { AgentCountsStatus } from "./AgentCountsStatus";

afterEach(() => {
    cleanup();
    open.mockClear();
});

const agent = (id: string, state: AgentRuntime["state"]): AgentRuntime => ({
    agent_id: id,
    state,
    desired: "running",
    name: id,
    provider: "claude",
    icon: "",
});

describe("AgentCountsStatus", () => {
    it("shows nothing while no agent runs", () => {
        const { container } = render(() => (
            <AgentCountsStatus rows={() => [agent("a", "stopped"), agent("b", "signed_out")]} />
        ));
        expect(container.querySelector(".status-agent-counts")).toBeNull();
    });

    it("counts the running agents, and says how many wait on the user", () => {
        render(() => (
            <AgentCountsStatus
                rows={() => [agent("a", "working"), agent("b", "idle"), agent("c", "waiting_on_user")]}
            />
        ));
        expect(screen.getByText("3 running")).toBeTruthy();
        expect(screen.getByText("1 waiting on you")).toBeTruthy();
        expect(screen.getByRole("button").getAttribute("data-tip")).toBe(
            "3 agents are running, 1 waiting on you. Click to open the Swarm"
        );
    });

    it("leaves the waiting count out when nobody waits, and follows the rows", () => {
        const [rows, setRows] = createSignal([agent("a", "idle")]);
        render(() => <AgentCountsStatus rows={rows} />);
        expect(screen.getByText("1 running")).toBeTruthy();
        expect(screen.queryByText(/waiting on you/)).toBeNull();
        expect(screen.getByRole("button").getAttribute("data-tip")).toBe(
            "1 agent is running. Click to open the Swarm"
        );
        setRows([agent("a", "waiting_on_user")]);
        expect(screen.getByText("1 waiting on you")).toBeTruthy();
        setRows([]);
        expect(screen.queryByRole("button")).toBeNull();
    });

    it("opens the Swarm on a click", () => {
        render(() => <AgentCountsStatus rows={() => [agent("a", "working")]} />);
        fireEvent.click(screen.getByRole("button"));
        expect(open).toHaveBeenCalledWith("swarm");
    });
});
