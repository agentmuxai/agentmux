// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const order: string[] = [];
let blockMeta: Record<string, unknown> | undefined;
// Whether this window has the block yet: when not, only a reload gives it.
let blockCached = true;
const reload = vi.fn(async (id: string) => {
    order.push(`reload:${id}`);
    return { meta: blockMeta };
});
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => "active" },
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getObjectValue: () => (blockCached ? { meta: blockMeta } : undefined),
        reloadMuxObject: (id: string) => reload(id),
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
let revealResult: { ok: boolean; reason?: string; detail?: string } = { ok: true };
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
            // The tab is read again after the attach, so the reveal sees the block the attach put in it.
            expect(order).toEqual(["note:a1", "rpc:tab-old", "reload:tab-old", "reveal:blk-1"]);
            expect(reveal).toHaveBeenCalledWith("blk-1", { tabId: "tab-old", source: "open", quiet: true });
        });

        it("is found even when this window has not loaded its block yet", async () => {
            blockCached = false;
            await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(reload).toHaveBeenCalledWith("blk-1");
            expect(order).toEqual(["note:a1", "reload:blk-1", "rpc:tab-old", "reload:tab-old", "reveal:blk-1"]);
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

        it("whose workspace cannot be shown says so, since the pane is back in it and the active tab can't take it", async () => {
            revealResult = { ok: false, reason: "remote-failed", detail: "workspace-closed" };
            const why = await openAgentInActiveTab({ agent_id: "a1", name: "Ada" });
            expect(why).toMatch(/could not be opened/);
            expect(order).toEqual(["note:a1", "rpc:tab-old", "reload:tab-old", "reveal:blk-1"]);
        });

        // The bug this guards: the pane came back (the attach worked) but the reveal was taken over by a
        // newer request, and the person was told the workspace could not be opened in a window. The newer
        // request is the one showing the pane, so the older one saying nothing is right.
        it("whose reveal a newer request took over reports no error: that request shows the pane", async () => {
            revealResult = { ok: false, reason: "superseded" };
            expect(await openAgentInActiveTab({ agent_id: "a1", name: "Ada" })).toBeUndefined();
        });

        // Every other failure is still said, but in the reveal's own words: never "the workspace could not
        // be opened in a window" unless that is what happened. Silent would leave Open doing nothing.
        it("whose reveal fails another way says why in the reveal's words, not the workspace's", async () => {
            for (const [reason, text] of [
                ["layout-timeout", "its tab is still loading"],
                ["mismatch", "it did not come to the front"],
                ["not-found", "it is no longer open"],
            ] as const) {
                revealResult = { ok: false, reason };
                expect(await openAgentInActiveTab({ agent_id: "a1", name: "Ada" })).toBe(text);
            }
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
