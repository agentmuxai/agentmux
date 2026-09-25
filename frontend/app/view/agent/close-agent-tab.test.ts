// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * closeAgentTab — SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §2.
 * Closing the last tab of this pane while it holds a loaded agent swaps in a
 * fresh My Agents tab; every other close is the ordinary closeBlockInStack.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
// vi.mock calls below are hoisted above this import (see open-history-tab.test.ts).
import { closeAgentTab } from "./close-agent-tab";

type FakeNode = { id: string; data: { blockId: string; blockStack?: string[] } };
let nodes: FakeNode[] = [];
const getNodeByBlockId = (blockId: string): FakeNode | undefined =>
    nodes.find((n) => (n.data.blockStack?.length ? n.data.blockStack : [n.data.blockId]).includes(blockId));

// Asked before the swap (the busy-agent close confirmation); true = go ahead.
const beforeNodeDelete = vi.fn(async (_data: unknown) => true);
const layoutModel = { getNodeByBlockId: vi.fn(getNodeByBlockId), beforeNodeDelete };
const addWidgetAsPaneTab = vi.fn();
const closeBlockInStack = vi.fn();
vi.mock("@/layout/index", () => ({
    addWidgetAsPaneTab: (...args: unknown[]) => addWidgetAsPaneTab(...args),
    closeBlockInStack: (...args: unknown[]) => closeBlockInStack(...args),
}));

const metaByBlock: Record<string, Record<string, unknown>> = {};
const agentBlockDef = { meta: { view: "agent", controller: "cmd" } };
vi.mock("@/app/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ widgets: { "defwidget@agent": { blockdef: agentBlockDef } } }) },
    MOS: {
        getObjectValue: (oref: string) => ({ meta: metaByBlock[oref.replace(/^block:/, "")] }),
        makeORef: (kind: string, id: string) => `${kind}:${id}`,
    },
}));

const holdLeafRevealGate = vi.fn(() => 7);
const scheduleLeafRevealLift = vi.fn();
vi.mock("@/app/store/tab-reveal", () => ({
    holdLeafRevealGate: (...args: unknown[]) => holdLeafRevealGate(...(args as [])),
    scheduleLeafRevealLift: (...args: unknown[]) => scheduleLeafRevealLift(...args),
}));

// Mirrors the real addWidgetAsPaneTab: pushes the new block onto the stack.
function pushNewBlock(nodeId: string, newBlockId: string): void {
    const n = nodes.find((x) => x.id === nodeId)!;
    const stack = n.data.blockStack?.length ? n.data.blockStack : [n.data.blockId];
    n.data.blockStack = [...stack, newBlockId];
    n.data.blockId = newBlockId;
}

describe("closeAgentTab", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        nodes = [];
        for (const k of Object.keys(metaByBlock)) delete metaByBlock[k];
        addWidgetAsPaneTab.mockImplementation(async (_model: unknown, nodeId: string) =>
            pushNewBlock(nodeId, "picker-1")
        );
        closeBlockInStack.mockResolvedValue(undefined);
        beforeNodeDelete.mockImplementation(async () => true);
    });

    it("last tab with a loaded agent → adds a My Agents tab, then closes the old one (pane survives)", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });

        expect(addWidgetAsPaneTab).toHaveBeenCalledTimes(1);
        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(layoutModel, "node-A", agentBlockDef);
        expect(closeBlockInStack).toHaveBeenCalledTimes(1);
        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-A", "agent-1", { confirmed: true });
        // The add happened BEFORE the close — otherwise the close would hit
        // the one-member branch and destroy the pane.
        expect(addWidgetAsPaneTab.mock.invocationCallOrder[0]).toBeLessThan(
            closeBlockInStack.mock.invocationCallOrder[0]
        );
        expect(nodes[0].data.blockStack).toEqual(["agent-1", "picker-1"]);
        expect(holdLeafRevealGate).toHaveBeenCalledWith("node-A");
        expect(scheduleLeafRevealLift).toHaveBeenCalledWith("node-A", 7);
    });

    it("last tab that is a History tab (agentId + historyTabFor) → also returns to My Agents", async () => {
        nodes = [{ id: "node-A", data: { blockId: "hist-1" } }]; // no blockStack: a one-member leaf
        metaByBlock["hist-1"] = { view: "agent", agentId: "def-1", "agent:historyTabFor": "def-1" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "hist-1" });

        expect(addWidgetAsPaneTab).toHaveBeenCalledTimes(1);
        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-A", "hist-1", { confirmed: true });
    });

    it("last tab already on My Agents (no agentId) → closes the pane as today", async () => {
        nodes = [{ id: "node-A", data: { blockId: "picker-0", blockStack: ["picker-0"] } }];
        metaByBlock["picker-0"] = { view: "agent" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "picker-0" });

        expect(addWidgetAsPaneTab).not.toHaveBeenCalled();
        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-A", "picker-0");
        expect(holdLeafRevealGate).not.toHaveBeenCalled();
    });

    it("last tab that isn't an agent (e.g. a terminal in an agent-started pane) → closes the pane as today", async () => {
        nodes = [{ id: "node-A", data: { blockId: "term-1", blockStack: ["term-1"] } }];
        metaByBlock["term-1"] = { view: "term" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "term-1" });

        expect(addWidgetAsPaneTab).not.toHaveBeenCalled();
        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-A", "term-1");
    });

    it("non-last agent tab → ordinary close (neighbor activates), no My Agents tab", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1", "agent-2"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });

        expect(addWidgetAsPaneTab).not.toHaveBeenCalled();
        expect(closeBlockInStack).toHaveBeenCalledTimes(1);
        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-A", "agent-1");
        expect(beforeNodeDelete).not.toHaveBeenCalled(); // closeBlockInStack asks
    });

    it("fork pill owned by ANOTHER pane, last tab there → closes that fork/pane as today", async () => {
        nodes = [
            { id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } },
            { id: "node-B", data: { blockId: "fork-1", blockStack: ["fork-1"] } },
        ];
        metaByBlock["fork-1"] = { view: "agent", agentId: "def-1-fork" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "fork-1" });

        expect(addWidgetAsPaneTab).not.toHaveBeenCalled();
        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-B", "fork-1");
    });

    it("pane.open fails → falls back to closing the pane, and lifts the reveal gate", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };
        addWidgetAsPaneTab.mockRejectedValue(new Error("rpc down"));
        vi.spyOn(console, "error").mockImplementation(() => {});

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });

        expect(closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-A", "agent-1", { confirmed: true });
        expect(nodes[0].data.blockStack).toEqual(["agent-1"]); // → closeBlockInStack's one-member branch
        expect(scheduleLeafRevealLift).toHaveBeenCalledWith("node-A", 7);
    });

    it("pane closed while pane.open was in flight → no close attempted on a vanished node", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };
        addWidgetAsPaneTab.mockImplementation(async () => {
            nodes = []; // pane gone; the real addWidgetAsPaneTab deletes its orphan block
        });

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });

        expect(closeBlockInStack).not.toHaveBeenCalled();
    });

    it("a double-click on × while the first close is in flight adds only one My Agents tab", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };
        let release!: () => void;
        addWidgetAsPaneTab.mockImplementation(
            (_m: unknown, nodeId: string) =>
                new Promise<void>((resolve) => {
                    release = () => {
                        pushNewBlock(nodeId, "picker-1");
                        resolve();
                    };
                })
        );

        const first = closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });
        const second = closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });
        // The confirmation is awaited first, so the add starts a tick later.
        await vi.waitFor(() => expect(addWidgetAsPaneTab).toHaveBeenCalled());
        release();
        await Promise.all([first, second]);

        expect(addWidgetAsPaneTab).toHaveBeenCalledTimes(1);
        expect(closeBlockInStack).toHaveBeenCalledTimes(1);
    });

    it("asks the busy-agent confirmation BEFORE adding the My Agents tab", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });

        expect(beforeNodeDelete).toHaveBeenCalledTimes(1);
        expect(beforeNodeDelete).toHaveBeenCalledWith({ blockId: "agent-1" });
        expect(beforeNodeDelete.mock.invocationCallOrder[0]).toBeLessThan(
            addWidgetAsPaneTab.mock.invocationCallOrder[0]
        );
    });

    it("cancelling the confirmation leaves the pane exactly as it was — no stray My Agents tab", async () => {
        nodes = [{ id: "node-A", data: { blockId: "agent-1", blockStack: ["agent-1"] } }];
        metaByBlock["agent-1"] = { view: "agent", agentId: "def-1" };
        beforeNodeDelete.mockImplementation(async () => false);

        await closeAgentTab({ layoutModel: layoutModel as any, ownNodeId: "node-A", blockId: "agent-1" });

        expect(addWidgetAsPaneTab).not.toHaveBeenCalled();
        expect(closeBlockInStack).not.toHaveBeenCalled();
        expect(holdLeafRevealGate).not.toHaveBeenCalled();
        expect(nodes[0].data.blockStack).toEqual(["agent-1"]);
    });
});
