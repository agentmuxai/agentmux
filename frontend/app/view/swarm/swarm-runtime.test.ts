// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { describe, expect, it } from "vitest";
import { agentsWithoutRow } from "./swarm-runtime";

const row = (agent_id: string, state: AgentRuntime["state"], extra: Partial<AgentRuntime> = {}): AgentRuntime => ({
    agent_id,
    state,
    desired: state === "stopped" || state === "error" ? "stopped" : "running",
    name: agent_id,
    provider: "claude",
    icon: "",
    ...extra,
});

describe("agentsWithoutRow", () => {
    it("lists every agent no live row shows, by name", () => {
        const rows = [
            row("z", "stopped", { name: "Zed" }),
            row("a", "error", { name: "alpha" }),
            row("p", "stopped", { name: "parked", block_id: "b-parked" }),
            row("l", "stopped", { name: "listed", block_id: "b-listed" }),
        ];
        expect(agentsWithoutRow(rows, new Set(["b-listed"])).map((r) => r.name)).toEqual(["alpha", "parked", "Zed"]);
    });

    it("lists a running agent the live rows have none for, whatever its state: it is never in neither", () => {
        const rows = [
            row("w", "working", { name: "Working", block_id: "b-w" }),
            row("i", "idle", { name: "Idle", block_id: "b-i" }),
            row("q", "waiting_on_user", { name: "Asking", block_id: "b-q" }),
            row("s", "starting", { name: "Starting" }),
            row("shown", "working", { name: "Shown", block_id: "b-shown" }),
        ];
        expect(agentsWithoutRow(rows, new Set(["b-shown"])).map((r) => r.name)).toEqual([
            "Asking",
            "Idle",
            "Starting",
            "Working",
        ]);
    });
});
