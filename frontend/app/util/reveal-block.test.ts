// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * revealBlock in this window — the order of steps that brings a block on screen.
 * SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md §4.1, §6.
 *
 * `setActiveBlockInStack` itself (switching a pane's tab) is covered by
 * layout/tests/layoutStack.test.ts; here it's recorded, so the test pins what
 * revealBlock adds: that it is called, for the right pane, before the
 * pane is focused and before the caret moves.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let ws: { oid: string; pinnedtabids: string[]; tabids: string[] } | null;
let tabs: Record<string, { blockids: string[] }>;
let models: Record<string, any>;
/** Slow-path doubles: srv's block.reveal, the host window API, legacy fallback. */
let blockReveal: (d: { block_id: string; await_ack?: boolean; open_if_closed?: boolean }) => Promise<{
    found: boolean;
    window_id?: string;
    revealed?: boolean;
    reason?: string;
    workspace_id?: string;
}>;
/** Opening a window on a workspace no window shows: made to fail by a test. */
let openWorkspaceInNewWindow: (workspaceId: string) => Promise<string>;
let otherWorkspaces: any[];
/** The window tab the window shows, and the notices the selection machine pushed. */
const win = vi.hoisted(() => ({ activeTab: "t1", commitDelayMs: 0 }));
const notices: { title: string; message: string }[] = [];
const windowInstances = [
    { label: "main", windowId: "win-here" },
    { label: "second", windowId: "win-other" },
];

vi.mock("@/app/store/global", () => ({
    workspace: () => ws,
    MOS: {
        makeORef: (_t: string, id: string) => id,
        getObjectValue: (id: string) => tabs[id],
        // Tabs, workspaces and windows share one id space here (makeORef returns the id).
        reloadMuxObject: async (id: string) => {
            if (id === "win-stale") throw new Error("not found: window win-stale");
            return (
                tabs[id] ??
            otherWorkspaces.find((w) => w.workspacedata.oid === id)?.workspacedata ??
                otherWorkspaces.filter((w) => w.windowid === id).map((w) => ({ workspaceid: w.workspacedata.oid }))[0]
            );
        },
    },
    atoms: { activeTabId: () => win.activeTab },
    pushNotification: (n: { title: string; message: string }) => notices.push(n),
    getApi: () => ({
        listWindowInstances: async () => windowInstances,
        focusWindow: async (label: string) => calls.push(`focusWindow:${label}`),
        openWorkspaceInNewWindow: (workspaceId: string) => openWorkspaceInNewWindow(workspaceId),
    }),
}));
vi.mock("@/app/store/tab-actions", () => ({
    setActiveTab: async (tabId: string) => {
        calls.push(`setActiveTab:${tabId}`);
        // The workspace push that commits the tab can land after the RPC replies.
        if (win.commitDelayMs) setTimeout(() => (win.activeTab = tabId), win.commitDelayMs);
        else win.activeTab = tabId;
    },
}));
vi.mock("@/app/store/focusManager", () => ({ giveBlockFocus: (id: string) => calls.push(`caret:${id}`) }));
/** The locate flash requests, each with the steps already taken when it was asked. */
const located: { blockId: string; after: string[] }[] = [];
vi.mock("@/app/util/pane-locate", () => ({
    emitPaneLocate: (blockId: string) => located.push({ blockId, after: [...calls] }),
}));
vi.mock("@/layout/index", () => ({
    setActiveBlockInStack: (_m: unknown, nodeId: string, blockId: string) => calls.push(`stack:${nodeId}:${blockId}`),
}));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: (id: string) => models[id] }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockRevealCommand: async (_c: unknown, d: { block_id: string; await_ack?: boolean; open_if_closed?: boolean }) => {
            calls.push(`block.reveal:${d.block_id}`);
            return blockReveal(d);
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({
    WorkspaceService: {
        SetActiveTab: async (ws: string, tab: string) => calls.push(`SetActiveTab:${ws}:${tab}`),
    },
    // The window list; each window's workspace and its tabs come from MOS (reloadMuxObject above).
    // A stale id first: session restore can leave one in Client.windowids.
    ClientService: {
        GetClientData: async () => ({ windowids: ["win-stale", "win-this", ...otherWorkspaces.map((w) => w.windowid)] }),
    },
}));

import { isWorkspaceClosedFailure } from "@/app/selection/reveal-failure";
import { findOpenBlockByView, revealBlock, showBlockInPane, showBlockWithoutFocus } from "./reveal-block";

/** A layout model whose one pane holds `members` as tabs. */
function modelWithPane(nodeId: string, members: string[], magnified?: string, minimized = false) {
    // Restoring a pane clears its flag, as the real toggle does (the machine reads it back).
    const flags = { minimized };
    return {
        magnifiedNodeId: magnified,
        treeState: { focusedNodeId: nodeId },
        getNodeByBlockId: (b: string) =>
            members.includes(b) ? { id: nodeId, ...(flags.minimized ? { minimized: true } : {}) } : null,
        focusNode: (id: string) => calls.push(`focusNode:${id}`),
        magnifyNodeToggle: (id: string, setState = true, focused = true) =>
            calls.push(`unmagnify:${id}${setState ? "" : ":nocommit"}${focused ? ":focus" : ""}`),
        minimizeNodeToggle: (id: string) => {
            flags.minimized = false;
            calls.push(`restore:${id}`);
        },
    };
}

beforeEach(() => {
    calls.length = 0;
    located.length = 0;
    notices.length = 0;
    win.activeTab = "t1";
    win.commitDelayMs = 0;
    ws = { oid: "ws", pinnedtabids: [], tabids: ["t1", "t2"] };
    tabs = { t1: { blockids: ["a"] }, t2: { blockids: ["agentA", "agentB"] } };
    models = { t1: modelWithPane("n1", ["a"]), t2: modelWithPane("pane", ["agentA", "agentB"]) };
    blockReveal = async () => ({ found: false });
    openWorkspaceInNewWindow = async (workspaceId) => {
        calls.push(`openWorkspaceInNewWindow:${workspaceId}`);
        return "window-label";
    };
    otherWorkspaces = [];
});
afterEach(() => vi.useRealTimers());

describe("revealBlock in this window only", () => {
    it("a background tab in a multi-tab pane: switch the window tab, then the pane's tab, then focus, then caret", async () => {
        expect((await revealBlock("agentB", { localOnly: true })).ok).toBe(true);
        expect(calls).toEqual(["setActiveTab:t2", "stack:pane:agentB", "focusNode:pane", "caret:agentB"]);
    });

    it("finds the tab through Tab.blockids, not a loaded layout, and waits for the layout to appear", async () => {
        vi.useFakeTimers();
        const late = modelWithPane("pane", ["agentA", "agentB"]);
        models.t2 = undefined; // not built until the tab renders
        const p = revealBlock("agentB", { localOnly: true });
        await vi.advanceTimersByTimeAsync(60);
        models.t2 = late;
        await vi.advanceTimersByTimeAsync(200); // the next look, then the settle before the read-back
        expect((await p).ok).toBe(true);
        expect(calls).toContain("stack:pane:agentB");
    });

    it("un-magnifies a different pane that would hide the target, before switching and focusing", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], "otherPane");
        await revealBlock("agentB", { localOnly: true });
        expect(calls.indexOf("unmagnify:otherPane")).toBeGreaterThan(-1);
        expect(calls.indexOf("unmagnify:otherPane")).toBeLessThan(calls.indexOf("stack:pane:agentB"));
    });

    it("leaves the target's own magnify alone", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], "pane");
        await revealBlock("agentB", { localOnly: true });
        expect(calls.some((c) => c.startsWith("unmagnify"))).toBe(false);
    });

    it("asks for the locate flash last, once the pane is switched to, focused and has its caret", async () => {
        await revealBlock("agentB", { localOnly: true });
        expect(located).toEqual([
            { blockId: "agentB", after: ["setActiveTab:t2", "stack:pane:agentB", "focusNode:pane", "caret:agentB"] },
        ]);
    });

    it("still asks for the flash when the caret is skipped", async () => {
        await revealBlock("agentB", { focusCaret: false, localOnly: true });
        expect(located.map((l) => l.blockId)).toEqual(["agentB"]);
    });

    it("asks for no flash when the block isn't in this window", async () => {
        await revealBlock("elsewhere", { localOnly: true });
        expect(located).toEqual([]);
    });

    it("focusCaret: false skips the caret", async () => {
        await revealBlock("agentB", { focusCaret: false, localOnly: true });
        expect(calls).not.toContain("caret:agentB");
    });

    it("a block not in this window returns false and touches nothing", async () => {
        expect((await revealBlock("elsewhere", { localOnly: true })).ok).toBe(false);
        expect(calls).toEqual([]);
        expect(calls.some((c) => c.startsWith("block.reveal"))).toBe(false); // no other window is asked
        expect(notices).toEqual([]); // the caller has the answer; nothing to tell the user
    });

    it("a this-window-only request that fails for another reason still says so", async () => {
        models.t2 = undefined; // the tab's layout never arrives
        vi.useFakeTimers();
        const p = revealBlock("agentB", { localOnly: true });
        await vi.advanceTimersByTimeAsync(2000);
        expect((await p).ok).toBe(false);
        expect(notices.map((n) => n.message)).toEqual(["its tab is still loading"]);
    });

    it("a tabId hint is tried first", async () => {
        tabs.t1 = { blockids: ["agentB"] }; // also claims agentB
        models.t1 = modelWithPane("n1", ["agentB"]);
        await revealBlock("agentB", { tabId: "t2", localOnly: true });
        expect(calls[0]).toBe("setActiveTab:t2");
    });

    it("restores a minimized pane before focusing it (focusNode alone leaves it collapsed)", async () => {
        models.t2 = modelWithPane("pane", ["agentA", "agentB"], undefined, true);
        await revealBlock("agentB", { localOnly: true });
        expect(calls).toEqual([
            "setActiveTab:t2",
            "restore:pane",
            "stack:pane:agentB",
            "focusNode:pane",
            "caret:agentB",
        ]);
    });
});

describe("findOpenBlockByView — an open pane the component registry does not list", () => {
    beforeEach(() => {
        // Block objects share the tab id space of the MOS double.
        tabs.m1 = { meta: { view: "memory" } } as never;
        tabs.m2 = { meta: { view: "memory" } } as never;
        tabs.t1 = { blockids: ["a", "m1"] };
        tabs.t2 = { blockids: ["agentA", "agentB", "m2"] };
    });

    it("finds an inactive tab of a pane's stack by its block's view", async () => {
        expect(await findOpenBlockByView("memory")).toBe("m1");
    });

    it("prefers the shown window tab, then looks in the others", async () => {
        win.activeTab = "t2";
        expect(await findOpenBlockByView("memory")).toBe("m2");
        tabs.t2 = { blockids: ["agentA", "agentB"] };
        expect(await findOpenBlockByView("memory")).toBe("m1");
        win.activeTab = "t1";
        tabs.t1 = { blockids: ["a"] };
        tabs.t2 = { blockids: ["agentA", "m2"] };
        expect(await findOpenBlockByView("memory")).toBe("m2");
    });

    it("is null when no pane of that view is open", async () => {
        expect(await findOpenBlockByView("settings")).toBeNull();
    });

    it("finds the pane of one agent, not the first agent pane", async () => {
        tabs.agentA = { meta: { view: "agent", agentId: "a" } } as never;
        tabs.agentB = { meta: { view: "agent", agentId: "b" } } as never;
        expect(await findOpenBlockByView("agent", "agent:b")).toBe("agentB");
        expect(await findOpenBlockByView("agent", "agent:a")).toBe("agentA");
        expect(await findOpenBlockByView("agent", "agent:c")).toBeNull();
        expect(await findOpenBlockByView("agent")).toBe("agentA");
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
        blockReveal = async () => ({ found: true, window_id: "win-other", revealed: true });
        expect(await revealBlock("remote")).toEqual({ ok: true });
        expect(calls).toEqual(["block.reveal:remote", "focusWindow:second"]);
    });

    it("waits for the other window's answer: it showed the pane, could not, or did not say", async () => {
        let asked: boolean | undefined;
        blockReveal = async (d) => ((asked = d.await_ack), { found: true, window_id: "win-other", revealed: false, reason: "layout-timeout" });
        expect(await revealBlock("remote")).toMatchObject({ ok: false, reason: "remote-failed", detail: "layout-timeout" });
        expect(asked).toBe(true);
        expect(notices.map((n) => n.message)).toEqual(["the other window could not show it"]);
        blockReveal = async () => ({ found: true, window_id: "win-other" });
        expect(await revealBlock("remote")).toMatchObject({ ok: false, reason: "remote-timeout" });
    });

    it("a block in this window never reaches srv", async () => {
        await revealBlock("agentB");
        expect(calls.some((c) => c.startsWith("block.reveal"))).toBe(false);
    });

    it("resolves settled for a block it showed, in order, and flashes it once it is read back", async () => {
        expect(await revealBlock("agentB", { source: "swarm" })).toEqual({ ok: true });
        expect(calls).toEqual(["setActiveTab:t2", "stack:pane:agentB", "focusNode:pane", "caret:agentB"]);
        expect(located.map((l) => l.blockId)).toEqual(["agentB"]);
        expect(notices).toEqual([]);
    });

    it("waits for a tab whose commit lands after the switch replied, instead of reading it back too early", async () => {
        win.commitDelayMs = 150;
        expect(await revealBlock("agentB")).toEqual({ ok: true });
        expect(win.activeTab).toBe("t2");
        expect(notices).toEqual([]);
    });

    it("resolves with the reason, and says it quietly, for a block that is nowhere", async () => {
        const result = await revealBlock("nowhere", { source: "swarm" });
        expect(result).toMatchObject({ ok: false, reason: "not-found" });
        expect(notices).toEqual([{ title: "Couldn't show that pane", message: "it is no longer open", icon: "fa-triangle-exclamation", timestamp: expect.any(String), type: "info", expiration: expect.any(Number), debug: expect.any(Object) }]);
        // What the toast's Copy button adds: the pane, the code, the source and the recent selection steps.
        expect((notices[0] as NotificationType).debug).toMatchObject({
            blockId: "nowhere",
            code: "reveal:not-found",
            detail: { source: "swarm", reason: "not-found", selectionTrace: expect.any(Array) },
        });
        expect(located).toEqual([]);
    });

    it("a pane that does not come to the front is shown once more, then reported", async () => {
        models.t2 = { ...modelWithPane("pane", ["agentA", "agentB"]), treeState: { focusedNodeId: "another" } };
        const result = await revealBlock("agentB");
        expect(result).toMatchObject({ ok: false, reason: "mismatch", detail: "the pane is not the focused one" });
        expect(calls.filter((c) => c === "stack:pane:agentB").length).toBe(2);
        expect(located).toEqual([]);
        expect(notices[0]?.message).toBe("it did not come to the front");
    });

    it("not found: falls back to activating its tab in the other workspace and raising that window", async () => {
        tabs.t9 = { blockids: ["remote"] };
        otherWorkspaces = [{ workspacedata: { oid: "ws2", name: "", pinnedtabids: [], tabids: ["t9"] }, windowid: "win-other" }];
        await revealBlock("remote");
        expect(calls).toEqual(["block.reveal:remote", "SetActiveTab:ws2:t9", "focusWindow:second"]);
    });

    it("a pane in a workspace no window has open: opens a window on that workspace, and says nothing", async () => {
        blockReveal = async (d) => {
            expect(d.open_if_closed).toBe(true);
            return { found: false, workspace_id: "ws-closed" };
        };
        const result = await revealBlock("masty", { source: "picker" });
        expect(result).toMatchObject({ ok: true });
        expect(calls).toEqual(["block.reveal:masty", "openWorkspaceInNewWindow:ws-closed"]);
        expect(notices).toEqual([]);
    });

    it("a pane whose workspace an open window holds is shown there, not in a second window", async () => {
        tabs.t9 = { blockids: ["remote"] };
        otherWorkspaces = [{ workspacedata: { oid: "ws2", name: "", pinnedtabids: [], tabids: ["t9"] }, windowid: "win-other" }];
        blockReveal = async () => ({ found: false, workspace_id: "ws2" });
        await revealBlock("remote");
        expect(calls).toEqual(["block.reveal:remote", "SetActiveTab:ws2:t9", "focusWindow:second"]);
    });

    it("a window that can't be opened on the workspace says so instead of 'no longer open'", async () => {
        blockReveal = async () => ({ found: false, workspace_id: "ws-closed" });
        openWorkspaceInNewWindow = async () => {
            throw new Error("host refused");
        };
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const result = await revealBlock("masty", { source: "picker" });
        warn.mockRestore();
        expect(result).toMatchObject({ ok: false, reason: "remote-failed", detail: "workspace-closed" });
        expect(notices[0]?.message).toBe("it is in a workspace no window has open, and a window could not be opened on it");
    });

    it("an older srv without block.reveal: the same fallback", async () => {
        blockReveal = async () => {
            throw new Error("unknown command block.reveal");
        };
        tabs.t9 = { blockids: ["remote"] };
        otherWorkspaces = [{ workspacedata: { oid: "ws2", name: "", pinnedtabids: [], tabids: ["t9"] }, windowid: "win-other" }];
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        await revealBlock("remote");
        warn.mockRestore();
        expect(calls).toEqual(["block.reveal:remote", "SetActiveTab:ws2:t9", "focusWindow:second"]);
    });

    // The shape of a parked agent's Open when this window's copy of the tab was read before the attach
    // reached it: the request goes to srv, which names this same window, whose `block:reveal` handler asks
    // again here. The second request ends the first as superseded — and the second is the one that shows
    // the pane, so the first caller must not call that a failure to open the workspace.
    it("a newer request from this window ends an older handed-over one as superseded, while it shows the pane", async () => {
        let answer!: (v: { found: boolean }) => void;
        blockReveal = () => new Promise((resolve) => (answer = resolve));
        const first = revealBlock("remote", { source: "open", quiet: true });
        await vi.waitFor(() => expect(calls).toContain("block.reveal:remote"));
        // srv told this window to reveal it (it holds the block now): the receiving side asks here, local only.
        tabs.t1 = { blockids: ["remote"] };
        models.t1 = modelWithPane("n1", ["remote"]);
        expect(await revealBlock("remote", { tabId: "t1", localOnly: true, quiet: true })).toEqual({ ok: true });
        expect(await first).toEqual({ ok: false, reason: "superseded" });
        answer({ found: false }); // the late answer finds no one waiting
    });
});

describe("isWorkspaceClosedFailure", () => {
    it("is true only for the reveal whose answer is that no window could be opened on the workspace", () => {
        expect(isWorkspaceClosedFailure({ ok: false, reason: "remote-failed", detail: "workspace-closed" })).toBe(true);
        for (const other of [
            { ok: true },
            { ok: false, reason: "remote-failed", detail: "layout-timeout" },
            { ok: false, reason: "remote-timeout" },
            { ok: false, reason: "superseded" },
            { ok: false, reason: "layout-timeout" },
            { ok: false, reason: "mismatch" },
        ] as const) {
            expect(isWorkspaceClosedFailure(other as any)).toBe(false);
        }
    });
});
