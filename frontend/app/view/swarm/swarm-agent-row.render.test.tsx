// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The PR #2862 regression, pinned end-to-end through the actual component.
 *
 * Both reviewers independently found that an agent whose ONLY activity was a
 * promoted Bash/sleep call rendered nothing: the long-running rows weren't in
 * `totalRows`, so `hasChildren()` was false, so there was no expand affordance,
 * so the collapse gate never mounted the bucket. `agentChildRowCount`'s unit
 * tests cover the sum; these cover the wiring the sum feeds.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { __resetAllSlots, dispatch, registerPane } from "@/app/store/agent-document-store";
import { AgentRow } from "./swarm-view";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import type { DocumentNode } from "@/app/view/agent/types";

afterEach(() => {
    cleanup();
    __resetAllSlots();
});

const BLOCK = "b1";

/** A pane whose only activity is a bare sleep — promoted immediately. */
function seedSleepingPane() {
    registerPane(BLOCK, () => {});
    const node: DocumentNode = {
        type: "tool",
        id: "t1",
        tool: "Bash",
        toolName: "Bash",
        status: "running",
        timestamp: Date.now(),
        params: { command: "sleep 300" },
        collapsed: false,
        summary: "",
    } as DocumentNode;
    dispatch(BLOCK, { type: "StreamFlush", newNodes: [node], updatedNodes: [] }, "system");
}

function treeNode(): AgentTreeNode {
    return {
        blockId: BLOCK,
        agentName: "AgentX",
        agentProvider: "claude",
        activitySummary: null,
        contextTokens: null,
        agentStatus: "running",
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        todoRows: [],
        todosTruncated: 0,
        currentTool: null,
    } as AgentTreeNode;
}

function modelStub(collapsed: boolean): SwarmViewModel {
    return {
        isAgentCollapsed: () => collapsed,
        toggleAgentCollapsed: () => {},
        isSelected: () => false,
        toggleSelected: () => {},
    } as unknown as SwarmViewModel;
}

function renderRow(collapsed: boolean) {
    return render(() => (
        <AgentRow node={treeNode()} focusedBlockId={() => null} model={modelStub(collapsed)} />
    ));
}

describe("AgentRow — an agent whose only activity is a long-running tool call", () => {
    it("shows the collapsed count, so the work is discoverable without expanding", () => {
        seedSleepingPane();
        const { container } = renderRow(true);
        expect(container.querySelector(".swarm-agent-collapsed-count")?.textContent).toBe("1");
    });

    it("renders the bucket and the row once expanded", () => {
        seedSleepingPane();
        const { container } = renderRow(false);
        expect(container.querySelector(".swarm-bucket--longrunning")).not.toBeNull();
        expect(container.querySelector(".swarm-longrunning-title")?.textContent).toBe("sleep 300");
    });

    it("shows the countdown, since a whole-command sleep knows its remaining time", () => {
        seedSleepingPane();
        const { container } = renderRow(false);
        expect(container.querySelector(".swarm-longrunning-remaining")?.textContent).toMatch(/^~\d+s left$/);
    });

    it("renders no bucket for an agent with nothing running", () => {
        registerPane(BLOCK, () => {});
        const { container } = renderRow(false);
        expect(container.querySelector(".swarm-bucket--longrunning")).toBeNull();
    });
});
