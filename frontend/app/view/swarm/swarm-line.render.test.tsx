// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm row's line, through the actual component: never empty, and a
 * fallback is visibly one (dimmer, with a tooltip) so it is not read as the
 * agent's own account of its work.
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.4.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));

import { __resetAllSlots } from "@/app/store/agent-document-store";
import { FALLBACK_TOOLTIP, RESTORED_TOOLTIP, type SwarmLine } from "@/app/store/swarm-line";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { AgentRow } from "./swarm-view";

afterEach(() => {
    cleanup();
    __resetAllSlots();
});

function treeNode(line: SwarmLine): AgentTreeNode {
    return {
        blockId: "b1",
        agentName: "AgentX",
        agentProvider: "claude",
        activitySummary: line.source === "generated" ? line.text : null,
        line,
        contextTokens: null,
        agentStatus: "idle",
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        todoRows: [],
        todosTruncated: 0,
        todosPartial: false,
        currentTool: null,
        backgroundTasks: NO_BACKGROUND_TASKS,
    } as AgentTreeNode;
}

const model = {
    isAgentCollapsed: () => false,
    toggleAgentCollapsed: () => {},
    isSelected: () => false,
    toggleSelected: () => {},
} as unknown as SwarmViewModel;

function line(l: SwarmLine) {
    const { container } = render(() => (
        <div class="swarm-view" tabIndex={-1}>
            <AgentRow node={treeNode(l)} focusedBlockId={() => null} model={model} />
        </div>
    ));
    return container.querySelector(".swarm-activity-summary") as HTMLElement | null;
}

describe("the Swarm row's line", () => {
    it("shows the agent's own title plainly, with no tooltip", () => {
        const el = line({ text: "Fix the login race", source: "generated" })!;
        expect(el.textContent).toBe("Fix the login race");
        expect(el.classList.contains("swarm-activity-summary--fallback")).toBe(false);
        expect(el.getAttribute("title")).toBeNull();
    });

    it("shows a status phrase for an agent with no title, dimmed and labelled", () => {
        const el = line({ text: "No activity yet", source: "status" })!;
        expect(el.textContent).toBe("No activity yet");
        expect(el.classList.contains("swarm-activity-summary--fallback")).toBe(true);
        expect(el.getAttribute("title")).toBe(FALLBACK_TOOLTIP);
    });

    it("labels a heuristic line as a fallback and a restored one as the previous session's", () => {
        expect(line({ text: "Fix the login redirect", source: "heuristic" })!.getAttribute("title")).toBe(FALLBACK_TOOLTIP);
        cleanup();
        const restored = line({ text: "Set up CI for the docs site", source: "restored" })!;
        expect(restored.getAttribute("title")).toBe(RESTORED_TOOLTIP);
        expect(restored.classList.contains("swarm-activity-summary--fallback")).toBe(true);
    });
});
