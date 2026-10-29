// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { agentsByRecentUse } from "./kanban-agent-order";
import type { KanbanAgent } from "./kanban-types";

const agent = (name: string): KanbanAgent => ({ name, block_id: `b-${name}`, state: "idle" });

describe("agentsByRecentUse", () => {
    const active: Record<string, number> = { Korp: 300, Opaz: 900, Parlo: 0, Clare: 0 };
    const read = (a: KanbanAgent) => active[a.name] ?? 0;

    it("puts the agent that produced output last first, and never-active agents last by name", () => {
        const sorted = agentsByRecentUse([agent("Parlo"), agent("Korp"), agent("Clare"), agent("Opaz")], read);
        expect(sorted.map((a) => a.name)).toEqual(["Opaz", "Korp", "Clare", "Parlo"]);
    });

    it("leaves its input alone", () => {
        const input = [agent("Korp"), agent("Opaz")];
        agentsByRecentUse(input, read);
        expect(input.map((a) => a.name)).toEqual(["Korp", "Opaz"]);
    });
});
