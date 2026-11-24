// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The row of the Trigger card that says which agents may fire the drone without asking: who made it, the agents it
 * was given to, and a way to give it to another or take it back. The list is srv's; only RPC is mocked.
 */
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const state = { owner: "opaz", agents: ["korp"] as string[] };
const get = vi.fn(() => Promise.resolve({ owner: state.owner, agents: [...state.agents] }));
const set = vi.fn((_c: unknown, req: { drone_id: string; agents: string[] }) => {
    state.agents = req.agents;
    return Promise.resolve({ owner: state.owner, agents: [...state.agents] });
});
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { GetDroneAccessCommand: (...a: unknown[]) => (get as any)(...a), SetDroneAgentsCommand: (...a: unknown[]) => (set as any)(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/view/agent/use-agent-list", () => ({
    useAgentList: () => [
        () => [
            { id: "opaz", name: "Opaz", is_seeded: false },
            { id: "korp", name: "Korp", is_seeded: false },
            { id: "clamk", name: "Clamk", is_seeded: false },
        ],
        () => null,
    ],
}));

import { DroneFireAgents } from "./drone-fire-agents";

afterEach(() => {
    cleanup();
    get.mockClear();
    set.mockClear();
    state.owner = "opaz";
    state.agents = ["korp"];
});

describe("the agents that may fire a drone", () => {
    it("shows who made it and who it was given to, and reads nothing for a drone that is not saved", async () => {
        const { container } = render(() => <DroneFireAgents droneId={() => "d1"} />);
        await waitFor(() => expect(container.querySelector(".drone-trigger-agents")).not.toBeNull());
        const text = container.querySelector(".drone-trigger-agents")!.textContent!;
        expect(text).toContain("made by opaz");
        expect(text).toContain("Stop korp firing it");
        cleanup();
        get.mockClear();
        const none = render(() => <DroneFireAgents droneId={() => ""} />);
        expect(none.container.querySelector(".drone-trigger-agents")).toBeNull();
        expect(get).not.toHaveBeenCalled();
    });

    it("takes a drone back from an agent it was given to", async () => {
        const { container } = render(() => <DroneFireAgents droneId={() => "d1"} />);
        await waitFor(() => expect(container.querySelector(".drone-trigger-agents")).not.toBeNull());
        const stop = [...container.querySelectorAll("button")].find((b) => b.textContent === "Stop korp firing it")!;
        fireEvent.click(stop);
        await waitFor(() => expect(set).toHaveBeenCalledWith({}, { drone_id: "d1", agents: [] }));
        await waitFor(() => expect(container.textContent).not.toContain("Stop korp firing it"));
    });

    it("reads the list again when another drone is opened in the same row", async () => {
        const lists: Record<string, { owner: string; agents: string[] }> = {
            d1: { owner: "opaz", agents: ["korp"] },
            d2: { owner: "", agents: [] },
        };
        get.mockImplementation(((_c: unknown, req: { drone_id: string }) => Promise.resolve(lists[req.drone_id])) as any);
        const [id, setId] = createSignal("d1");
        const { container } = render(() => <DroneFireAgents droneId={id} />);
        await waitFor(() => expect(container.textContent).toContain("Stop korp firing it"));
        setId("d2");
        await waitFor(() => expect(container.textContent).not.toContain("Stop korp firing it"));
        expect(get).toHaveBeenLastCalledWith({}, { drone_id: "d2" });
        get.mockImplementation((() => Promise.resolve({ owner: state.owner, agents: [...state.agents] })) as any);
    });

    it("says an agent will ask first when nobody made the drone and no agent was given it", async () => {
        state.owner = "";
        state.agents = [];
        const { container } = render(() => <DroneFireAgents droneId={() => "d1"} />);
        await waitFor(() => expect(container.querySelector(".drone-trigger-note")?.textContent).toContain("asks you first"));
    });
});
