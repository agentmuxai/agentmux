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
/** Slow-path doubles: srv's block.reveal, the host window API, legacy fallback. */
let blockReveal: (d: { block_id: string }) => Promise<{ found: boolean; window_id?: string }>;
let otherWorkspaces: any[];
const windowInstances = [
    { label: "main", windowId: "win-here" },
    { label: "second", windowId: "win-other" },
];

vi.mock("@/app/store/global", () => ({
    workspace: () => ws,
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getObjectValue: (id: string) => tabs[id],
        reloadMuxObject: async (id: string) => tabs[id],
    },
    setActiveTab: async (tabId: string) => calls.push(`setActiveTab:${tabId}`),
    getApi: () => ({
        listWindowInstances: async () => windowInstances,
        focusWindow: async (label: string) => calls.push(`focusWindow:${label}`),
    }),
}));
vi.mock("@/app/store/focusManager", () => ({ giveBlockFocus: (id: string) => calls.push(`caret:${id}`) }));
vi.mock("@/layout/index", () => ({
    setActiveBlockInStack: (_m: unknown, nodeId: string, blockId: string) => calls.push(`stack:${nodeId}:${blockId}`),
}));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: (id: string) => models[id] }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockRevealCommand: async (_c: unknown, d: { block_id: string }) => {
            calls.push(`block.reveal:${d.block_id}`);
            return blockReveal(d);
        },
        WorkspaceListCommand: async () => otherWorkspaces,
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({
    WorkspaceService: { SetActiveTab: async (ws: string, tab: string) => calls.push(`SetActiveTab:${ws}:${tab}`) },
}));

import { revealBlock, revealBlockLocally, showBlockInPane, showBlockWithoutFocus } from "./reveal-block";

/** A layout model whose one pane holds `members` as tabs. */
function modelWithPane(nodeId: string, members: string[], magnified?: string, minimized = false) {
    return {
        magnifiedNodeId: magnified,
        getNodeByBlockId: (b: string) =>
            members.includes(b) ? { id: nodeId, ...(minimized ? { minimized: true } : {}) } : null,
        focusNode: (id: string) => calls.push(`focusNode:${id}`),
        magnifyNodeToggle: (id: string, setState = true, focused = true) =>
            calls.push(`unmagnify:${id}${setState ? "" : ":nocommit"}${focused ? ":focus" : ""}`),
        minimizeNodeToggle: (id: string) => calls.push(`restore:${id}`),
    };
}

beforeEach(() => {
    calls.length = 0;
    ws = { oid: "ws", pinnedtabids: [], tabids: ["t1", "t2"] };
    tabs = { t1: { blockids: ["a"] }, t2: { blockids: ["agentA", "agentB"] } };
    models = { t1: modelWithPane("n1", ["a"]), t2: modelWithPane("pane", ["agentA", "agentB"]) };
    blockReveal = async () => ({ found: false });
    otherWorkspaces = [];
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

    it("restores a minimized pane before focusing it (focusNode alone leaves it collapsed)", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], undefined, true);
        await revealBlockLocally("agentB");
        expect(calls).toEqual([
            "setActiveTab:t2",
            "restore:pane",
            "stack:pane:agentB",
            "focusNode:pane",
            "caret:agentB",
        ]);
    });
});

describe("showBlockInPane — visible in its layout, no focus", () => {
    const node = (id: string, minimized = false) => ({ id, ...(minimized ? { minimized: true } : {}) }) as any;

    it("minimized: restores it, then switches its pane to it", () => {
        showBlockInPane(modelWithPane("pane", ["a"]) as any, node("pane", true), "a");
        expect(calls).toEqual(["restore:pane", "stack:pane:a"]);
    });

    it("already visible: never calls the minimize toggle (it would minimize it)", () => {
        showBlockInPane(modelWithPane("pane", ["a"]) as any, node("pane"), "a");
        expect(calls).toEqual(["stack:pane:a"]);
    });

    it("another pane magnified: un-magnifies it; its own magnify is left alone", () => {
        showBlockInPane(modelWithPane("pane", ["a"], "other") as any, node("pane"), "a");
        expect(calls).toEqual(["unmagnify:other", "stack:pane:a"]);
        calls.length = 0;
        showBlockInPane(modelWithPane("pane", ["a"], "pane") as any, node("pane"), "a");
        expect(calls).toEqual(["stack:pane:a"]);
    });

    it("minimized behind a magnified sibling: both undone", () => {
        showBlockInPane(modelWithPane("pane", ["a"], "other") as any, node("pane", true), "a");
        expect(calls).toEqual(["restore:pane", "unmagnify:other", "stack:pane:a"]);
    });

    it("un-magnifies with a committed, focus-free toggle (a plain toggle requests focus)", () => {
        showBlockInPane(modelWithPane("pane", ["a"], "other") as any, node("pane"), "a");
        expect(calls).toContain("unmagnify:other");
        expect(calls.some((c) => c.endsWith(":focus") || c.endsWith(":nocommit"))).toBe(false);
    });

    it("never focuses, moves the caret or switches the window tab", () => {
        showBlockInPane(modelWithPane("pane", ["a"], "other") as any, node("pane", true), "a");
        expect(calls.some((c) => /^(focusNode|caret|setActiveTab)/.test(c))).toBe(false);
    });
});

describe("showBlockWithoutFocus — an agent's file-open into an existing pane", () => {
    it("finds the block's tab and shows it there without switching to that tab", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], "other", true);
        expect(await showBlockWithoutFocus("agentB")).toBe(true);
        expect(calls).toEqual(["restore:pane", "unmagnify:other", "stack:pane:agentB"]);
    });

    it("a block not in a built layout here: false, nothing touched", async () => {
        expect(await showBlockWithoutFocus("elsewhere")).toBe(false);
        models.t2 = undefined;
        expect(await showBlockWithoutFocus("agentB")).toBe(false);
        expect(calls).toEqual([]);
    });
});

describe("revealBlock — a block in another window (spec §4.3)", () => {
    it("asks srv to reveal it there, then raises the window srv named", async () => {
        blockReveal = async () => ({ found: true, window_id: "win-other" });
        await revealBlock("remote");
        expect(calls).toEqual(["block.reveal:remote", "focusWindow:second"]);
    });

    it("a block in this window never reaches srv", async () => {
        await revealBlock("agentB");
        expect(calls.some((c) => c.startsWith("block.reveal"))).toBe(false);
    });

    it("not found: falls back to activating its tab in the other workspace and raising that window", async () => {
        tabs.t9 = { blockids: ["remote"] };
        otherWorkspaces = [{ workspacedata: { oid: "ws2", pinnedtabids: [], tabids: ["t9"] }, windowid: "win-other" }];
        await revealBlock("remote");
        expect(calls).toEqual(["block.reveal:remote", "SetActiveTab:ws2:t9", "focusWindow:second"]);
    });

    it("an older srv without block.reveal: the same fallback", async () => {
        blockReveal = async () => {
            throw new Error("unknown command block.reveal");
        };
        tabs.t9 = { blockids: ["remote"] };
        otherWorkspaces = [{ workspacedata: { oid: "ws2", pinnedtabids: [], tabids: ["t9"] }, windowid: "win-other" }];
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        await revealBlock("remote");
        warn.mockRestore();
        expect(calls).toEqual(["block.reveal:remote", "SetActiveTab:ws2:t9", "focusWindow:second"]);
    });
});
