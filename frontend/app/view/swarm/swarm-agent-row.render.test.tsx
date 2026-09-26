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

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const focusBlockMock = vi.fn();
vi.mock("@/app/util/focus-block", () => ({ focusBlock: (...args: unknown[]) => focusBlockMock(...args) }));

import { __resetAllSlots, dispatch, registerPane } from "@/app/store/agent-document-store";
import { AgentRow } from "./swarm-view";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import type { DocumentNode } from "@/app/view/agent/types";

afterEach(() => {
    cleanup();
    __resetAllSlots();
    focusBlockMock.mockClear();
});

const BLOCK = "b1";

/** A pane whose only activity is a bare sleep — promoted immediately. */
function seedSleepingPane() {
    registerPane(BLOCK);
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
        todosPartial: false,
        currentTool: null,
    } as AgentTreeNode;
}

function modelStub(collapsed: boolean, toggleAgentCollapsed: () => void = () => {}): SwarmViewModel {
    return {
        isAgentCollapsed: () => collapsed,
        toggleAgentCollapsed,
        isSelected: () => false,
        toggleSelected: () => {},
    } as unknown as SwarmViewModel;
}

function renderRow(collapsed: boolean, model: SwarmViewModel = modelStub(collapsed)) {
    return render(() => (
        <AgentRow node={treeNode()} focusedBlockId={() => null} model={model} />
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
        registerPane(BLOCK);
        const { container } = renderRow(false);
        expect(container.querySelector(".swarm-bucket--longrunning")).toBeNull();
    });
});

/**
 * SPEC_SWARM_ROW_AGENT_COLOR_AND_SELECT_TO_FOCUS_2026_09_25.md §2.3 — the row
 * body now focuses the agent's own pane (not the Swarm pane, and not
 * collapse-toggle); the chevron alone still collapses.
 */
describe("AgentRow — select-to-focus", () => {
    it("clicking the row body focuses the agent's pane", () => {
        registerPane(BLOCK);
        const { container } = renderRow(false);

        fireEvent.click(container.querySelector(".swarm-agent-card")!);

        expect(focusBlockMock).toHaveBeenCalledExactlyOnceWith(BLOCK);
    });

    it("the row's click handler stops propagation, so it can't also reach the Swarm pane's own click-to-focus", () => {
        // Solid delegates click at the mount root and re-implements bubbling
        // among its own onClick handlers, so a plain native listener on an
        // ancestor doesn't observe Solid-level stopPropagation() the way
        // block.tsx's own (also Solid onClick) handler does in production —
        // asserting on the native Event object itself is the
        // delegation-agnostic way to check the row calls it.
        registerPane(BLOCK);
        const { container } = renderRow(false);
        const stopPropagationSpy = vi.spyOn(MouseEvent.prototype, "stopPropagation");

        fireEvent.click(container.querySelector(".swarm-agent-card")!);

        expect(stopPropagationSpy).toHaveBeenCalled();
        stopPropagationSpy.mockRestore();
    });

    it("clicking the row body does not toggle collapse", () => {
        seedSleepingPane(); // gives the row a chevron to collapse
        const toggle = vi.fn();
        const { container } = renderRow(false, modelStub(false, toggle));

        fireEvent.click(container.querySelector(".swarm-agent-card")!);

        expect(toggle).not.toHaveBeenCalled();
        expect(focusBlockMock).toHaveBeenCalledExactlyOnceWith(BLOCK);
    });

    it("clicking the chevron toggles collapse without focusing the pane", () => {
        seedSleepingPane();
        const toggle = vi.fn();
        const { container } = renderRow(false, modelStub(false, toggle));

        fireEvent.click(container.querySelector(".swarm-agent-expand-icon")!);

        expect(toggle).toHaveBeenCalledExactlyOnceWith(BLOCK);
        expect(focusBlockMock).not.toHaveBeenCalled();
    });
});
