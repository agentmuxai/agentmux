// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * revealBlockLocally — the order of steps that brings a block on screen.
 * SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md §4.1, §6.
 *
 * `setActiveBlockInStack` itself (switching a pane's tab) is covered by
 * layout/tests/layoutStack.test.ts; here it's recorded, so the test pins what
 * revealBlockLocally adds: that it is called, for the right pane, before the
 * pane is focused and before the caret moves.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let ws: { oid: string; pinnedtabids: string[]; tabids: string[] } | null;
let tabs: Record<string, { blockids: string[] }>;
let models: Record<string, any>;

vi.mock("@/app/store/global", () => ({
    workspace: () => ws,
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getObjectValue: (id: string) => tabs[id],
        reloadMuxObject: async (id: string) => tabs[id],
    },
    setActiveTab: async (tabId: string) => calls.push(`setActiveTab:${tabId}`),
    getApi: () => ({}),
}));
vi.mock("@/app/store/focusManager", () => ({ giveBlockFocus: (id: string) => calls.push(`caret:${id}`) }));
vi.mock("@/layout/index", () => ({
    setActiveBlockInStack: (_m: unknown, nodeId: string, blockId: string) => calls.push(`stack:${nodeId}:${blockId}`),
}));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: (id: string) => models[id] }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));

import { revealBlockLocally } from "./reveal-block";

/** A layout model whose one pane holds `members` as tabs. */
function modelWithPane(nodeId: string, members: string[], magnified?: string) {
    return {
        magnifiedNodeId: magnified,
        getNodeByBlockId: (b: string) => (members.includes(b) ? { id: nodeId } : null),
        focusNode: (id: string) => calls.push(`focusNode:${id}`),
        magnifyNodeToggle: (id: string) => calls.push(`unmagnify:${id}`),
    };
}

beforeEach(() => {
    calls.length = 0;
    ws = { oid: "ws", pinnedtabids: [], tabids: ["t1", "t2"] };
    tabs = { t1: { blockids: ["a"] }, t2: { blockids: ["agentA", "agentB"] } };
    models = { t1: modelWithPane("n1", ["a"]), t2: modelWithPane("pane", ["agentA", "agentB"]) };
});
afterEach(() => vi.useRealTimers());

describe("revealBlockLocally", () => {
    it("a background tab in a multi-tab pane: switch the window tab, then the pane's tab, then focus, then caret", async () => {
        expect(await revealBlockLocally("agentB")).toBe(true);
        expect(calls).toEqual(["setActiveTab:t2", "stack:pane:agentB", "focusNode:pane", "caret:agentB"]);
    });

    it("finds the tab through Tab.blockids, not a loaded layout, and waits for the layout to appear", async () => {
        vi.useFakeTimers();
        const late = modelWithPane("pane", ["agentA", "agentB"]);
        models.t2 = undefined; // not built until the tab renders
        const p = revealBlockLocally("agentB");
        await vi.advanceTimersByTimeAsync(60);
        models.t2 = late;
        await vi.advanceTimersByTimeAsync(60);
        expect(await p).toBe(true);
        expect(calls).toContain("stack:pane:agentB");
    });

    it("un-magnifies a different pane that would hide the target, before switching and focusing", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], "otherPane");
        await revealBlockLocally("agentB");
        expect(calls.indexOf("unmagnify:otherPane")).toBeGreaterThan(-1);
        expect(calls.indexOf("unmagnify:otherPane")).toBeLessThan(calls.indexOf("stack:pane:agentB"));
    });

    it("leaves the target's own magnify alone", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], "pane");
        await revealBlockLocally("agentB");
        expect(calls.some((c) => c.startsWith("unmagnify"))).toBe(false);
    });

    it("focusCaret: false skips the caret", async () => {
        await revealBlockLocally("agentB", { focusCaret: false });
        expect(calls).not.toContain("caret:agentB");
    });

    it("a block not in this window returns false and touches nothing", async () => {
        expect(await revealBlockLocally("elsewhere")).toBe(false);
        expect(calls).toEqual([]);
    });

    it("a tabId hint is tried first", async () => {
        tabs.t1 = { blockids: ["agentB"] }; // also claims agentB
        models.t1 = modelWithPane("n1", ["agentB"]);
        await revealBlockLocally("agentB", { tabId: "t2" });
        expect(calls[0]).toBe("setActiveTab:t2");
    });
});
