// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A Swarm row for an agent that runs with no pane carries the same "No pane" mark as its My Agents row. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { DockNodeStatusCommand: () => Promise.resolve() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const { revealBlock, openAgentInActiveTab } = vi.hoisted(() => ({
    revealBlock: vi.fn(),
    openAgentInActiveTab: vi.fn(),
}));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock }));
vi.mock("@/app/store/agent-runtime-live", () => ({ openAgentInActiveTab }));

import { __resetAllSlots, registerPane } from "@/app/store/agent-document-store";
import { MOS } from "@/app/store/global";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { AgentRow } from "./swarm-view";

beforeEach(() => {
    revealBlock.mockReset();
    openAgentInActiveTab.mockReset();
    openAgentInActiveTab.mockResolvedValue(undefined);
});
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

describe("AgentRow - clicking an agent with no pane", () => {
    const card = (container: HTMLElement) => container.querySelector<HTMLElement>(".swarm-agent-card")!;

    it("opens its pane in the active tab instead of trying to focus one, and says so on hover", () => {
        registerPane("b-click");
        seedBlockMeta("b-click", { "agent:parked": true, agentId: "tpl", agentInstanceId: "uid-1" });
        const { container } = renderRow("b-click");
        expect(card(container).title).toContain("click to open");
        fireEvent.click(card(container));
        expect(openAgentInActiveTab).toHaveBeenCalledTimes(1);
        expect(openAgentInActiveTab).toHaveBeenCalledWith({ agent_id: "uid-1", name: "Korp" });
        expect(revealBlock).not.toHaveBeenCalled();
    });

    it("is one open for a double click, and another can follow once it has ended", async () => {
        registerPane("b-dbl");
        seedBlockMeta("b-dbl", { "agent:parked": true, agentId: "uid-2" });
        const { container } = renderRow("b-dbl");
        fireEvent.click(card(container));
        fireEvent.click(card(container));
        expect(openAgentInActiveTab).toHaveBeenCalledTimes(1);
        await vi.waitFor(() => {
            fireEvent.click(card(container));
            expect(openAgentInActiveTab).toHaveBeenCalledTimes(2);
        });
    });

    it("opens from the keyboard too", () => {
        registerPane("b-key");
        seedBlockMeta("b-key", { "agent:parked": true, agentId: "uid-3" });
        const { container } = renderRow("b-key");
        fireEvent.keyDown(card(container), { key: "Enter" });
        expect(openAgentInActiveTab).toHaveBeenCalledTimes(1);
    });

    it("leaves the checkbox to itself", () => {
        registerPane("b-ctl");
        seedBlockMeta("b-ctl", { "agent:parked": true, agentId: "uid-4" });
        const { container } = renderRow("b-ctl");
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        expect(openAgentInActiveTab).not.toHaveBeenCalled();
    });

    it("focuses the pane when the agent has one by the time of the click", () => {
        registerPane("b-shown2");
        seedBlockMeta("b-shown2", { agentId: "uid-5" });
        const { container } = renderRow("b-shown2");
        expect(card(container).title).toBe("Korp");
        fireEvent.click(card(container));
        expect(revealBlock).toHaveBeenCalledWith("b-shown2", { source: "swarm" });
        expect(openAgentInActiveTab).not.toHaveBeenCalled();
    });

    it("says why srv refused, under the row, and drops it on the next try", async () => {
        registerPane("b-no");
        seedBlockMeta("b-no", { "agent:parked": true, agentId: "uid-6" });
        openAgentInActiveTab.mockResolvedValueOnce("this provider cannot run here");
        const { container } = renderRow("b-no");
        fireEvent.click(card(container));
        await vi.waitFor(() => expect(container.querySelector("[role=alert]")?.textContent).toBe("this provider cannot run here"));
        fireEvent.click(card(container));
        await vi.waitFor(() => expect(container.querySelector("[role=alert]")).toBeNull());
    });
});
