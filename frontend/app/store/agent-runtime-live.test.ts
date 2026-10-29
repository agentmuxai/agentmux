// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const order: string[] = [];
vi.mock("@/app/store/global", () => ({ atoms: { activeTabId: () => "t1" } }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn() }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { AgentsChanged: "agents:changed" } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let openResult: () => Promise<unknown> = async () => ({ block_id: "b1" });
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentOpenCommand: async () => {
            order.push("rpc");
            return openResult();
        },
    },
}));
vi.mock("@/app/store/agent-open-note", () => ({
    noteOpenedByPerson: (id: string) => order.push(`note:${id}`),
}));

import { openAgentInActiveTab } from "./agent-runtime-live";

describe("openAgentInActiveTab", () => {
    beforeEach(() => {
        order.length = 0;
        openResult = async () => ({ block_id: "b1" });
    });

    it("notes the open for the agent before it asks srv, since the pane can mount before the answer", async () => {
        await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
        expect(order).toEqual(["note:a1", "rpc"]);
    });

    it("still notes it, and says why, when srv refuses", async () => {
        openResult = async () => {
            throw new Error("open in another pane");
        };
        const why = await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
        expect(why).toBe("open in another pane");
        expect(order[0]).toBe("note:a1");
    });
});
