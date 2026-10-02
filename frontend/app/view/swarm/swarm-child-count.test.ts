// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `agentChildRowCount` drives the expand chevron, the collapsed-count badge and
 * `hasChildren` — so a bucket missing from the sum is invisible in the UI, not
 * merely miscounted. Both reviewers independently caught exactly that on
 * PR #2862 (the long-running bucket omitted). These tests enumerate every
 * bucket so the next omission fails here instead of shipping.
 */

import { describe, expect, it } from "vitest";
import { agentChildRowCount } from "./swarm-view";
import type { AgentTreeNode } from "./swarm-model";
import { NO_BACKGROUND_TASKS } from "./swarm-background";

function node(over: Partial<AgentTreeNode> = {}): AgentTreeNode {
    return {
        blockId: "b1",
        agentName: "AgentX",
        agentProvider: "claude",
        activitySummary: null,
        line: { text: "No activity yet", source: "status" },
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
        backgroundTasks: NO_BACKGROUND_TASKS,
        ...over,
    } as AgentTreeNode;
}

const one = [{} as never];

describe("agentChildRowCount", () => {
    it("is zero for an agent with nothing running", () => {
        expect(agentChildRowCount(node(), 0)).toBe(0);
    });

    /** The PR #2862 bug: an agent whose ONLY activity is a promoted Bash/sleep
     *  call must still report children, or it renders no chevron, stays
     *  collapsed, and the bucket never mounts. */
    it("counts a long-running tool call even when every other bucket is empty", () => {
        expect(agentChildRowCount(node(), 1)).toBe(1);
        expect(agentChildRowCount(node(), 3)).toBe(3);
    });

    it.each([
        ["agentToolRows", { agentToolRows: one }],
        ["workflowRows", { workflowRows: one }],
        ["shellRows", { shellRows: one }],
        ["cronRows", { cronRows: one }],
    ])("counts %s", (_label, over) => {
        expect(agentChildRowCount(node(over as Partial<AgentTreeNode>), 0)).toBe(1);
    });

    /** An agent whose only activity is a background command it (or a
     *  subagent without a row) launched must still report children, for the
     *  same reason as the long-running case above. */
    it("counts the agent-level Background bucket even when every other bucket is empty", () => {
        expect(agentChildRowCount(node(), 0, 1)).toBe(1);
        expect(agentChildRowCount(node(), 0, 4)).toBe(4);
    });

    it("sums every bucket together", () => {
        const n = node({ agentToolRows: one, workflowRows: one, shellRows: one, cronRows: one });
        expect(agentChildRowCount(n, 2)).toBe(6);
        expect(agentChildRowCount(n, 2, 3)).toBe(9);
    });
});
