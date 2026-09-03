// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Todo rows + the current-tool line, end-to-end through `AgentRow`.
 *
 * The backend parser has its own tests (`progress_watcher.rs`); these cover the
 * wiring those payloads feed — including the `agentChildRowCount` trap that the
 * PR #2862 regression documents: a bucket that isn't in the count renders
 * nothing and offers no way to expand to it.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { __resetAllSlots, registerPane } from "@/app/store/agent-document-store";
import { AgentRow, agentChildRowCount } from "./swarm-view";
import type { AgentTreeNode, SwarmViewModel, TodoItem } from "./swarm-model";

afterEach(() => {
    cleanup();
    __resetAllSlots();
});

const BLOCK = "b1";

function treeNode(over: Partial<AgentTreeNode> = {}): AgentTreeNode {
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
        ...over,
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

function renderRow(node: AgentTreeNode, collapsed = false) {
    registerPane(BLOCK, () => {});
    return render(() => <AgentRow node={node} focusedBlockId={() => null} model={modelStub(collapsed)} />);
}

const TODOS: TodoItem[] = [
    { text: "Read the spec", status: "completed" },
    { text: "Write the parser", status: "in_progress" },
    { text: "Wire the frontend", status: "pending" },
];

describe("Swarm todo rows", () => {
    it("lists every todo under the agent, in the agent's own order", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS }));
        const texts = [...container.querySelectorAll(".swarm-todo-text")].map((e) => e.textContent);
        expect(texts).toEqual(["Read the spec", "Write the parser", "Wire the frontend"]);
    });

    it("shows progress as done/total rather than a bare row count", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS }));
        expect(container.querySelector(".swarm-bucket--todo .swarm-bucket-count")?.textContent).toBe("1/3");
    });

    it("marks each row with its status so state is visible without reading", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS }));
        expect(container.querySelector(".swarm-todo-row--completed")).not.toBeNull();
        expect(container.querySelector(".swarm-todo-row--in_progress")).not.toBeNull();
        expect(container.querySelector(".swarm-todo-row--pending")).not.toBeNull();
    });

    /** A provider we haven't seen must still render its row, not vanish. */
    it("falls back to the pending treatment for an unrecognized status", () => {
        const { container } = renderRow(treeNode({ todoRows: [{ text: "Odd one", status: "deferred" }] }));
        expect(container.querySelector(".swarm-todo-row--pending")).not.toBeNull();
        expect(container.querySelector(".swarm-todo-text")?.textContent).toBe("Odd one");
    });

    it("renders no bucket at all when the agent has no todos", () => {
        const { container } = renderRow(treeNode());
        expect(container.querySelector(".swarm-bucket--todo")).toBeNull();
    });

    it("discloses backend truncation instead of showing a partial list as whole", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS, todosTruncated: 7 }));
        expect(container.querySelector(".swarm-todo-truncated")?.textContent).toBe("+7 more");
    });

    /** `todosPartial` and `todosTruncated` are different claims and must not
     *  look the same: one is a cap we chose and can count, the other is
     *  history the backend could not see. Rendering only the countable one
     *  would show an incomplete checklist as if it were whole. */
    it("discloses a partial checklist separately from a truncated one", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS, todosPartial: true }));
        expect(container.querySelector(".swarm-todo-partial")?.textContent).toBe(
            "earlier items may be missing"
        );
        expect(container.querySelector(".swarm-todo-truncated")).toBeNull();
    });

    it("says nothing about completeness when the checklist is complete", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS }));
        expect(container.querySelector(".swarm-todo-partial")).toBeNull();
    });

    it("can report both at once — they are independent facts", () => {
        const { container } = renderRow(
            treeNode({ todoRows: TODOS, todosTruncated: 2, todosPartial: true })
        );
        expect(container.querySelector(".swarm-todo-truncated")?.textContent).toBe("+2 more");
        expect(container.querySelector(".swarm-todo-partial")).not.toBeNull();
    });

    it("keeps the full text reachable on hover, since rows are single-line", () => {
        const long = "a".repeat(200);
        const { container } = renderRow(treeNode({ todoRows: [{ text: long, status: "pending" }] }));
        expect(container.querySelector(".swarm-todo-text")?.getAttribute("title")).toBe(long);
    });
});

describe("agentChildRowCount — todos must be countable", () => {
    /** The PR #2862 trap: a bucket missing from this sum gets no expand
     *  affordance and never mounts, so it renders as nothing at all. */
    it("counts todo rows", () => {
        expect(agentChildRowCount(treeNode({ todoRows: TODOS }), 0)).toBe(3);
    });

    it("renders the bucket for an agent whose ONLY activity is a checklist", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS }));
        expect(container.querySelector(".swarm-bucket--todo")).not.toBeNull();
    });

    it("surfaces that work in the collapsed count too", () => {
        const { container } = renderRow(treeNode({ todoRows: TODOS }), true);
        expect(container.querySelector(".swarm-agent-collapsed-count")?.textContent).toBe("3");
    });
});

describe("Swarm current-tool line", () => {
    it("names the tool in flight", () => {
        const { container } = renderRow(treeNode({ currentTool: "Bash" }));
        expect(container.querySelector(".swarm-current-tool-name")?.textContent).toBe("Bash");
    });

    /** It is the most perishable thing on the card — an agent you have
     *  collapsed is exactly when you want it without expanding. */
    it("stays visible while the agent is collapsed", () => {
        const { container } = renderRow(treeNode({ currentTool: "Bash" }), true);
        expect(container.querySelector(".swarm-current-tool-name")?.textContent).toBe("Bash");
    });

    it("shows nothing between tools", () => {
        const { container } = renderRow(treeNode({ currentTool: null }));
        expect(container.querySelector(".swarm-current-tool")).toBeNull();
    });
});
