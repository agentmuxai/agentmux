// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Background tasks in the Swarm, through the real AgentRow: the agent's own
 * tasks in its Background bucket, a subagent's tasks nested under that
 * subagent's row, and no duplicate in the transcript-derived Running bucket.
 * SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md §4.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/focus-block", () => ({ focusBlock: () => {} }));

import { __resetAllSlots, dispatch, registerPane } from "@/app/store/agent-document-store";
import type { BackgroundTaskView } from "@/app/store/rpc-api";
import type { DocumentNode } from "@/app/view/agent/types";
import { AgentRow } from "./swarm-view";
import type { ActiveSubagent, AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { groupBackgroundTasks } from "./swarm-background";

afterEach(() => {
    cleanup();
    __resetAllSlots();
});

const BLOCK = "b1";
const SPAWN = "toolu_spawn";

function task(overrides: Partial<BackgroundTaskView> = {}): BackgroundTaskView {
    return {
        id: "toolu_task",
        block_id: BLOCK,
        label: "Background wait for agents",
        pid: null,
        started_at_ms: Date.now() - 5_000,
        status: "running",
        last_seen_ms: Date.now() - 5_000,
        ended_at_ms: null,
        owner_tool_use_id: null,
        ...overrides,
    };
}

function subagent(): ActiveSubagent {
    return {
        agent_id: "a1",
        slug: "audit",
        parent_agent: "AgentX",
        parent_block_id: BLOCK,
        session_id: "s1",
        status: "active",
        spawned_at: Date.now() - 60_000,
        last_event_at: Date.now(),
        event_count: 3,
        model: null,
        dispatch_id: "solo:a1",
        display_name: "Audit macOS specs",
        tool_use_id: SPAWN,
    };
}

function treeNode(tasks: BackgroundTaskView[], subs: ActiveSubagent[] = []): AgentTreeNode {
    const shown = new Set(subs.map((s) => s.tool_use_id).filter((id): id is string => !!id));
    return {
        blockId: BLOCK,
        agentName: "AgentX",
        agentProvider: "claude",
        activitySummary: null,
        line: { text: "No activity yet", source: "status" },
        contextTokens: null,
        agentStatus: "running",
        agentToolRows: subs,
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        todoRows: [],
        todosTruncated: 0,
        todosPartial: false,
        currentTool: null,
        backgroundTasks: groupBackgroundTasks(tasks, BLOCK, shown),
    } as AgentTreeNode;
}

function modelStub(): SwarmViewModel {
    return {
        isAgentCollapsed: () => false,
        toggleAgentCollapsed: () => {},
        isSelected: () => false,
        toggleSelected: () => {},
        isExpanded: () => false,
        toggleDispatchExpanded: () => {},
        retireRow: () => {},
        pauseCountdown: () => {},
        resumeCountdown: () => {},
        countdownStateAtom: () => new Map(),
    } as unknown as SwarmViewModel;
}

function renderRow(node: AgentTreeNode) {
    registerPane(BLOCK);
    return render(() => <AgentRow node={node} focusedBlockId={() => null} model={modelStub()} />);
}

describe("Swarm background tasks", () => {
    it("shows the agent's own background task in its Background bucket, with its description and status", () => {
        const { container } = renderRow(treeNode([task()]));
        const bucket = container.querySelector(".swarm-bucket--background");
        expect(bucket).not.toBeNull();
        expect(bucket?.querySelector(".swarm-background-title")?.textContent).toBe("Background wait for agents");
        expect(bucket?.querySelector(".swarm-background-status")?.textContent).toBe("running");
    });

    it("an agent whose only activity is a background task still expands to show it", () => {
        const { container } = renderRow(treeNode([task()]));
        expect(container.querySelector(".swarm-children")).not.toBeNull();
    });

    it("nests a subagent's background task under that subagent's row, not in the agent's bucket", () => {
        const { container } = renderRow(treeNode([task({ owner_tool_use_id: SPAWN })], [subagent()]));
        const group = container.querySelector(".swarm-subagent-group");
        expect(group?.querySelector(".swarm-subagent-background .swarm-background-title")?.textContent).toBe(
            "Background wait for agents"
        );
        expect(container.querySelector(".swarm-bucket--background")).toBeNull();
    });

    it("shows a finished task as done for the retention window, then drops it", () => {
        const recent = task({ status: "done", ended_at_ms: Date.now() - 1_000 });
        const { container } = renderRow(treeNode([recent]));
        expect(container.querySelector(".swarm-background-status")?.textContent).toBe("done");
        cleanup();

        const old = task({ status: "done", ended_at_ms: Date.now() - 60_000 });
        const { container: later } = renderRow(treeNode([old]));
        expect(later.querySelector(".swarm-bucket--background")).toBeNull();
    });

    it("a background call the registry tracks is not shown again in the Running bucket", () => {
        registerPane(BLOCK);
        const running: DocumentNode = {
            type: "tool",
            id: "toolu_task",
            tool: "Bash",
            toolName: "Bash",
            status: "running",
            timestamp: Date.now() - 5_000,
            params: { command: "sleep 300" },
            collapsed: false,
            summary: "",
        } as DocumentNode;
        dispatch(BLOCK, { type: "StreamFlush", newNodes: [running], updatedNodes: [] }, "system");

        const { container } = renderRow(treeNode([task({ id: "toolu_task" })]));
        expect(container.querySelector(".swarm-bucket--longrunning")).toBeNull();
        expect(container.querySelectorAll(".swarm-background-row")).toHaveLength(1);
    });
});
