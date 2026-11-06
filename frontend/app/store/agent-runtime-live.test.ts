// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const order: string[] = [];
let blockMeta: Record<string, unknown> | undefined;
// Whether this window has the block yet: when not, only a reload gives it.
let blockCached = true;
const reload = vi.fn(async () => ({ meta: blockMeta }));
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => "t1" },
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getObjectValue: () => (blockCached ? { meta: blockMeta } : undefined),
        reloadMuxObject: () => reload(),
    },
}));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn() }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { AgentsChanged: "agents:changed" } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let openResult: () => Promise<unknown> = async () => ({ block_id: "b1" });
let runtimeRows: { agent_id: string; state: string; block_id?: string; desired: string; name: string; provider: string; icon: string }[] = [];
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentRuntimeListCommand: async () => runtimeRows,
        AgentOpenCommand: async () => {
            order.push("rpc");
            return openResult();
        },
    },
}));
vi.mock("@/app/store/agent-open-note", () => ({
    noteOpenedByPerson: (id: string) => order.push(`note:${id}`),
}));
let revealResult: { ok: boolean } = { ok: true };
const reveal = vi.fn(async (blockId: string, _opts: unknown) => {
    order.push(`reveal:${blockId}`);
    return revealResult;
});
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: (...a: [string, unknown]) => reveal(...a) }));

import { openAgentInActiveTab, sharedAgentRuntime } from "./agent-runtime-live";

const row = (over: Partial<(typeof runtimeRows)[number]> = {}) => ({
    agent_id: "a1",
    state: "idle",
    desired: "running",
    name: "Ada",
    provider: "claude",
    icon: "",
    ...over,
});

describe("openAgentInActiveTab", () => {
    beforeEach(() => {
        order.length = 0;
        reveal.mockClear();
        blockMeta = undefined;
        blockCached = true;
        reload.mockClear();
        revealResult = { ok: true };
        runtimeRows = [];
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

    describe("a parked agent", () => {
        beforeEach(async () => {
            runtimeRows = [row({ block_id: "blk-1" })];
            blockMeta = { "agent:parked": true };
            await sharedAgentRuntime().load();
        });

        it("whose workspace still exists is shown there, in a window with its pane focused, and not moved", async () => {
            const why = await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(why).toBeUndefined();
            expect(reveal).toHaveBeenCalledWith("blk-1", { source: "open", quiet: true });
            expect(order).toEqual(["note:a1", "reveal:blk-1"]);
        });

        it("is found even when this window has not loaded its block yet", async () => {
            blockCached = false;
            await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(reload).toHaveBeenCalledOnce();
            expect(reveal).toHaveBeenCalledWith("blk-1", { source: "open", quiet: true });
            expect(order).toEqual(["note:a1", "reveal:blk-1"]);
        });

        it("whose workspace is gone falls back to opening in the active tab, with no failure notice", async () => {
            revealResult = { ok: false };
            const why = await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(why).toBeUndefined();
            expect(order).toEqual(["note:a1", "reveal:blk-1", "rpc"]);
            expect(reveal).toHaveBeenCalledWith("blk-1", expect.objectContaining({ quiet: true }));
        });

        it("still says why when the fallback is refused", async () => {
            revealResult = { ok: false };
            openResult = async () => {
                throw new Error("provider can't run here");
            };
            expect(await openAgentInActiveTab({ agent_id: "a1", name: "Ada" })).toBe("provider can't run here");
        });
    });

    it("an agent whose block is not parked goes the usual way: srv opens it in the active tab", async () => {
        runtimeRows = [row({ block_id: "blk-1" })];
        blockMeta = {};
        await sharedAgentRuntime().load();
        await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
        expect(reveal).not.toHaveBeenCalled();
        expect(order).toEqual(["note:a1", "rpc"]);
    });

    it("an agent with no block goes the usual way", async () => {
        runtimeRows = [row()];
        await sharedAgentRuntime().load();
        await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
        expect(reveal).not.toHaveBeenCalled();
        expect(order).toEqual(["note:a1", "rpc"]);
    });
});
