// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The agents as palette entries: a name to find them by, their seat and state after it, and the words of
// both searchable. The search itself is `searchCommands`, run for real.

import { describe, expect, it, vi } from "vitest";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { AgentSeat } from "@/types/rpc/AgentSeat";
import { searchCommands } from "./command-palette-search";
import { agentDetail, agentEntries, seatText } from "./palette-agent-entries";

const row = (agent_id: string, name: string, state: AgentRuntime["state"] = "idle"): AgentRuntime => ({
    agent_id,
    name,
    state,
    provider: "claude",
    icon: "robot",
    desired: "running",
} as AgentRuntime);

const seat = (s: Pick<AgentSeat, "role" | "qualifier" | "area">): AgentSeat => ({ ...s, set_at_ms: 0 }) as AgentSeat;
const korpSeat = seat({ role: "specialist", area: "browser engine: CEF fork builds, hibernation" });
const camperSeat = seat({ role: "specialist", qualifier: "layout", area: "panes, layout, tabs, terminal, Swarm" });

describe("seatText and agentDetail", () => {
    it("reads a seat as its role and area, and an agent with no seat as nothing", () => {
        expect(seatText(korpSeat)).toBe("Specialist · browser engine: CEF fork builds, hibernation");
        expect(seatText(camperSeat)).toBe("Specialist · layout · panes, layout, tabs, terminal, Swarm");
        expect(seatText({ role: "worker" })).toBe("Worker");
        expect(seatText(undefined)).toBe("");
    });

    it("puts the seat, then the state", () => {
        expect(agentDetail({ state: "working" }, korpSeat)).toBe("Specialist · browser engine: CEF fork builds, hibernation · working");
        expect(agentDetail({ state: "waiting_on_user" }, undefined)).toBe("waiting on you");
    });
});

describe("agentEntries", () => {
    const seats = new Map<string, AgentSeat | undefined>([
        ["k", korpSeat],
        ["c", camperSeat],
    ]);
    const entries = (rows: AgentRuntime[], go = vi.fn()) => agentEntries(rows, seats, go);

    it("makes one entry per agent: its name, the Agent category, its detail and its icon", () => {
        const [korp] = entries([row("k", "Korp", "working")]);
        expect(korp).toMatchObject({
            id: "agent:k",
            label: "Korp",
            category: "Agent",
            icon: "robot",
            detail: "Specialist · browser engine: CEF fork builds, hibernation · working",
        });
    });

    it("shows an agent with no seat or icon by its name and state alone", () => {
        const [plain] = entries([{ ...row("x", "Plain", "stopped"), icon: "" }]);
        expect(plain.detail).toBe("stopped");
        expect(plain.icon).toBe("robot");
    });

    it("runs go with the agent's id and name when selected", () => {
        const go = vi.fn();
        entries([row("k", "Korp")], go)[0].execute();
        expect(go).toHaveBeenCalledWith({ agent_id: "k", name: "Korp" });
    });

    it("follows a rename and a new agent: it is built from the rows it is given", () => {
        expect(entries([row("k", "Korp")]).map((e) => e.label)).toEqual(["Korp"]);
        expect(entries([row("k", "Korp-2"), row("n", "Newcomer")]).map((e) => e.label)).toEqual(["Korp-2", "Newcomer"]);
    });

    it("is found by name, by a word of the seat or area, and by state", () => {
        const all = entries([row("k", "Korp", "working"), row("c", "Camper", "idle"), { ...row("n", "Naki", "limited"), provider: "codex" }]);
        expect(searchCommands(all, "korp").map((e) => e.label)).toEqual(["Korp"]);
        expect(searchCommands(all, "browser").map((e) => e.label)).toEqual(["Korp"]);
        expect(searchCommands(all, "terminal").map((e) => e.label)).toEqual(["Camper"]);
        expect(searchCommands(all, "limited").map((e) => e.label)).toEqual(["Naki"]);
        expect(searchCommands(all, "codex").map((e) => e.label)).toEqual(["Naki"]);
    });

    it("lists the agent named for the query before one that only mentions it", () => {
        const all = entries([row("c", "Camper"), { ...row("b", "Browser Helper"), name: "Browser Helper" }, row("k", "Korp")]);
        expect(searchCommands(all, "browser").map((e) => e.label)).toEqual(["Browser Helper", "Korp"]);
    });
});
