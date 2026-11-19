// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListAgentDefinitionsCommand: async () => [] } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/domain-store", () => ({
    createDomainStore: (o: { initial: unknown }) => ({ data: () => o.initial, follow: () => {} }),
}));

import { agentDisplayName, nameInAgents } from "./agent-names";

const agents = [
    { id: "def-1", slug: "tess-2", name: "Tess" },
    { id: "def-2", slug: "agent2-6", name: "Agent2-6" },
    { id: "def-3", slug: "nameless", name: "" },
];

describe("an agent as a person reads it", () => {
    it("is the agent's name for the id srv records it by, whatever the case", () => {
        expect(nameInAgents(agents, "tess-2")).toBe("Tess");
        expect(nameInAgents(agents, "TESS-2")).toBe("Tess");
        expect(nameInAgents(agents, "def-1")).toBe("Tess");
    });

    it("is the id itself where no agent has it, or the agent has no name, or the list is not loaded", () => {
        expect(nameInAgents(agents, "ghost")).toBe("ghost");
        expect(nameInAgents(agents, "nameless")).toBe("nameless");
        expect(nameInAgents([], "tess-2")).toBe("tess-2");
        expect(agentDisplayName("tess-2")).toBe("tess-2");
        expect(nameInAgents(agents, "")).toBe("");
    });
});
