// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const order: string[] = [];
let blockMeta: Record<string, unknown> | undefined;
// Whether this window has the block yet: when not, only a reload gives it.
let blockCached = true;
const reload = vi.fn(async () => ({ meta: blockMeta }));
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => "active" },
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
let runtimeRows: {
    agent_id: string;
    state: string;
    block_id?: string;
    desired: string;
    name: string;
    provider: string;
    icon: string;
}[] = [];
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentRuntimeListCommand: async () => runtimeRows,
        AgentOpenCommand: async (_c: unknown, p: { tab_id: string }) => {
            order.push(`rpc:${p.tab_id}`);
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
let tabs = new Set<string>();
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: (...a: [string, unknown]) => reveal(...a),
    loadObject: async (_t: string, id: string) => (tabs.has(id) ? { oid: id } : null),
}));

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
        tabs = new Set();
        blockMeta = undefined;
        blockCached = true;
        reload.mockClear();
        revealResult = { ok: true };
        runtimeRows = [];
        openResult = async () => ({ block_id: "b1" });
    });

    it("notes the open for the agent before it asks srv, since the pane can mount before the answer", async () => {
        await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
        expect(order).toEqual(["note:a1", "rpc:active"]);
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
            blockMeta = { "agent:parked": true, "agent:parkedFromTab": "tab-old" };
            tabs = new Set(["tab-old"]);
            await sharedAgentRuntime().load();
        });

        it("whose tab still exists is put back in it and shown there, in a window with its pane focused", async () => {
            const why = await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(why).toBeUndefined();
            expect(reveal).toHaveBeenCalledWith("blk-1", { source: "open", quiet: true });
            expect(order).toEqual(["note:a1", "rpc:tab-old", "reveal:blk-1"]);
        });

        it("is found even when this window has not loaded its block yet", async () => {
            blockCached = false;
            await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(reload).toHaveBeenCalledOnce();
            expect(order).toEqual(["note:a1", "rpc:tab-old", "reveal:blk-1"]);
        });

        it("whose tab is gone with its workspace opens in the active tab, with no failure notice", async () => {
            tabs = new Set();
            const why = await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(why).toBeUndefined();
            expect(order).toEqual(["note:a1", "rpc:active"]);
            expect(reveal).not.toHaveBeenCalled();
        });

        it("that remembers no tab opens in the active tab", async () => {
            blockMeta = { "agent:parked": true };
            await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(order).toEqual(["note:a1", "rpc:active"]);
        });

        it("whose workspace cannot be shown falls back to opening in the active tab", async () => {
            revealResult = { ok: false };
            await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(order).toEqual(["note:a1", "rpc:tab-old", "reveal:blk-1", "rpc:active"]);
        });

        it("still says why when srv refuses to put it back", async () => {
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
        expect(order).toEqual(["note:a1", "rpc:active"]);
    });

    it("an agent with no block goes the usual way", async () => {
        runtimeRows = [row()];
        await sharedAgentRuntime().load();
        await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
        expect(reveal).not.toHaveBeenCalled();
        expect(order).toEqual(["note:a1", "rpc:active"]);
    });
});
