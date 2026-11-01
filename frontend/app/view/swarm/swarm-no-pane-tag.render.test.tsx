// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A Swarm row for an agent that runs with no pane carries the same "No pane" mark as its My Agents row. */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { DockNodeStatusCommand: () => Promise.resolve() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));

import { __resetAllSlots, registerPane } from "@/app/store/agent-document-store";
import { MOS } from "@/app/store/global";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { AgentRow } from "./swarm-view";

afterEach(() => {
    cleanup();
    __resetAllSlots();
});

let version = 0;
function seedBlockMeta(blockId: string, meta: Record<string, unknown>): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid: blockId,
        obj: { otype: "block", oid: blockId, version: ++version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

const node = (blockId: string): AgentTreeNode =>
    ({
        blockId,
        agentName: "Korp",
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
    }) as AgentTreeNode;

const model = {
    isAgentCollapsed: () => false,
    toggleAgentCollapsed: () => {},
    isSelected: () => false,
    toggleSelected: () => {},
} as unknown as SwarmViewModel;

const renderRow = (blockId: string) =>
    render(() => (
        <div class="swarm-view" tabIndex={-1}>
            <AgentRow node={() => node(blockId)} focusedBlockId={() => null} model={model} />
        </div>
    ));

describe("AgentRow — No pane", () => {
    it("tags an agent whose block is parked", () => {
        registerPane("b-parked");
        seedBlockMeta("b-parked", { "agent:parked": true });
        const { container } = renderRow("b-parked");
        expect(container.querySelector(".swarm-agent-row")?.textContent).toContain("No pane");
    });

    it("tags nothing on an agent shown in a pane", () => {
        registerPane("b-shown");
        seedBlockMeta("b-shown", { agentId: "uid-x" });
        const { container } = renderRow("b-shown");
        expect(container.querySelector(".swarm-agent-row")?.textContent).not.toContain("No pane");
    });
});
