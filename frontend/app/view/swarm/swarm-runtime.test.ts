// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { describe, expect, it } from "vitest";
import { stoppedAgents } from "./swarm-runtime";

const row = (agent_id: string, state: AgentRuntime["state"], extra: Partial<AgentRuntime> = {}): AgentRuntime => ({
    agent_id,
    state,
    desired: state === "stopped" || state === "error" ? "stopped" : "running",
    name: agent_id,
    provider: "claude",
    icon: "",
    ...extra,
});

describe("stoppedAgents", () => {
    it("lists the agents with no process that no live row shows, by name", () => {
        const rows = [
            row("z", "stopped", { name: "Zed" }),
            row("a", "error", { name: "alpha" }),
            row("m", "idle", { name: "mid", block_id: "b-live" }),
            row("p", "stopped", { name: "parked", block_id: "b-parked" }),
            row("l", "stopped", { name: "listed", block_id: "b-listed" }),
        ];
        expect(stoppedAgents(rows, new Set(["b-listed"])).map((r) => r.name)).toEqual(["alpha", "parked", "Zed"]);
    });
});
