// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm row for an agent that can't start a turn (out of quota until a reset, or signed
 * out), and the note for a tool call that has been running a while.
 * docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §4.4.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));

import { __resetAllSlots, dispatch, registerPane } from "@/app/store/agent-document-store";
import { formatResetTime } from "@/app/store/agent-unavailable";
import { MOS } from "@/app/store/global";
import type { DocumentNode } from "@/app/view/agent/types";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { remoteChip, type RemoteRow } from "./swarm-remote";
import { AgentRow, RemoteAgentRow } from "./swarm-view";

afterEach(() => {
    cleanup();
    __resetAllSlots();
});

const BLOCK = "unavailable-b1";
let version = 0;

/** A newer version each time: the store ignores an update that isn't. */
function seedBlockMeta(meta: Record<string, unknown>): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid: BLOCK,
        obj: { otype: "block", oid: BLOCK, version: ++version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

/** A pane whose only activity is a bare sleep that started `minutesAgo` minutes ago. */
function seedSleepingPane(minutesAgo: number) {
    registerPane(BLOCK);
    const node = {
        type: "tool",
        id: "t1",
        tool: "Bash",
        toolName: "Bash",
        status: "running",
        timestamp: Date.now() - minutesAgo * 60_000,
        params: { command: "sleep 3000" },
        collapsed: false,
        summary: "",
    } as DocumentNode;
    dispatch(BLOCK, { type: "StreamFlush", newNodes: [node], updatedNodes: [] }, "system");
}

function treeNode(agentStatus: "running" | "idle"): AgentTreeNode {
    return {
        blockId: BLOCK,
        agentName: "AgentX",
        agentProvider: "claude",
        activitySummary: null,
        line: { text: "No activity yet", source: "status" },
        contextTokens: null,
        agentStatus,
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
    isAgentCollapsed: () => true,
    toggleAgentCollapsed: () => {},
    isSelected: () => false,
    toggleSelected: () => {},
} as unknown as SwarmViewModel;

function renderRow(agentStatus: "running" | "idle") {
    return render(() => (
        <div class="swarm-view" tabIndex={-1}>
            <AgentRow node={() => treeNode(agentStatus)} focusedBlockId={() => null} model={model} />
        </div>
    ));
}

const hours = (n: number) => Date.now() + n * 3_600_000;

describe("AgentRow — an agent that can't start a turn", () => {
    it("shows out of quota with the reset instead of the status chip", () => {
        const resets = hours(3);
        seedBlockMeta({ "agent:last_failure": { code: "usage_limit", resetsAtMs: resets, limitWindow: "five_hour" } });
        const { container } = renderRow("idle");
        const chip = container.querySelector(".swarm-unavailable-chip");
        expect(chip?.textContent?.trim()).toBe(`Out of quota · until ${formatResetTime(resets, Date.now())}`);
        expect(chip?.getAttribute("title")).toContain("five hour limit");
        expect(container.querySelector(".swarm-status-chip")).toBeNull();
    });

    it("shows out of quota with no time when the provider gave none", () => {
        seedBlockMeta({ "agent:last_failure": { code: "usage_limit" } });
        const { container } = renderRow("idle");
        expect(container.querySelector(".swarm-unavailable-chip")?.textContent?.trim()).toBe("Out of quota");
    });

    it("shows signed out for a refused sign-in", () => {
        seedBlockMeta({ "agent:last_failure": { code: "auth" } });
        const { container } = renderRow("idle");
        expect(container.querySelector(".swarm-unavailable-chip--signed_out")?.textContent?.trim()).toBe("Signed out");
    });

    it("says it in the row's line too, ahead of a title", () => {
        seedBlockMeta({ "agent:last_failure": { code: "auth" } });
        // The line is resolved by the model when it builds the row; the chip is what the row owns.
        const { container } = renderRow("idle");
        expect(container.querySelector(".swarm-unavailable-chip")).not.toBeNull();
    });

    it("keeps the status chip for an agent mid-turn, whatever its last failure", () => {
        seedBlockMeta({ "agent:last_failure": { code: "usage_limit", resetsAtMs: hours(3) } });
        const { container } = renderRow("running");
        expect(container.querySelector(".swarm-unavailable-chip")).toBeNull();
        expect(container.querySelector(".swarm-status-chip")).not.toBeNull();
    });

    it("is gone once the reset has passed", () => {
        seedBlockMeta({ "agent:last_failure": { code: "usage_limit", resetsAtMs: Date.now() - 1000 } });
        const { container } = renderRow("idle");
        expect(container.querySelector(".swarm-unavailable-chip")).toBeNull();
        expect(container.querySelector(".swarm-status-chip")).not.toBeNull();
    });

    it("leaves a Supervisor pause ahead of it: the row keeps the line the model built, which says why it needs you", () => {
        seedBlockMeta({
            "agent:last_failure": { code: "usage_limit", resetsAtMs: hours(3) },
            "supervisor:mode": "steady",
            "supervisor:state": {
                status: "paused",
                reason: "out of quota until Sun 7 PM",
                mode: "steady",
                history: [],
            },
        });
        const { container } = renderRow("idle");
        expect(container.querySelector(".swarm-unavailable-chip")).toBeNull();
        expect(container.querySelector(".swarm-status-chip")).not.toBeNull();
        expect(container.querySelector(".swarm-activity-summary")?.textContent?.trim()).toBe("No activity yet");
    });

    it("ignores a transient failure", () => {
        seedBlockMeta({ "agent:last_failure": { code: "overloaded" } });
        const { container } = renderRow("idle");
        expect(container.querySelector(".swarm-unavailable-chip")).toBeNull();
    });
});

describe("AgentRow — a tool call that has been running a while", () => {
    it("says what is running and for how many minutes, on the row", () => {
        seedBlockMeta({});
        seedSleepingPane(12);
        const { container } = renderRow("running");
        expect(container.querySelector(".swarm-long-tool")?.textContent).toBe("Running sleep 3000 · 12 min");
    });

    it("says nothing under a minute, or for an agent that is not mid-turn", () => {
        seedBlockMeta({});
        seedSleepingPane(0);
        expect(renderRow("running").container.querySelector(".swarm-long-tool")).toBeNull();
        cleanup();
        __resetAllSlots();
        seedSleepingPane(12);
        expect(renderRow("idle").container.querySelector(".swarm-long-tool")).toBeNull();
    });

    it("is information only: the status chip stays as it was", () => {
        seedBlockMeta({});
        seedSleepingPane(30);
        const { container } = renderRow("running");
        expect(container.querySelector(".swarm-status-chip--working")).not.toBeNull();
        expect(container.querySelector(".swarm-unavailable-chip")).toBeNull();
    });
});

describe("a remote agent that can't start a turn", () => {
    const section = {
        key: "lan:area54#1/stable",
        title: "Area54",
        badge: "LAN",
        tier: "lan",
        platform: "macOS",
        stale: false,
        seenAtMs: Date.now(),
        agents: [],
    } as unknown as RemoteRow["section"];
    const row = (status: string): RemoteRow => ({
        section,
        agent: { name: "Manoz", block_id: "", status } as RemoteRow["agent"],
    });

    it("gets a chip that says out of quota or signed out, not idle", () => {
        expect(remoteChip("limited", false)).toMatchObject({ unavailable: "limited", label: "Out of quota" });
        expect(remoteChip("signed_out", false)).toMatchObject({ unavailable: "signed_out", label: "Signed out" });
        expect(remoteChip("limited", true)).toBeNull();
    });

    it("shows that chip on the row", () => {
        const { container } = render(() => (
            <div>
                <RemoteAgentRow model={model} row={row("limited")} />
                <RemoteAgentRow
                    model={model}
                    row={{
                        ...row("signed_out"),
                        agent: { name: "Korp", block_id: "", status: "signed_out" } as RemoteRow["agent"],
                    }}
                />
            </div>
        ));
        const chips = [...container.querySelectorAll(".swarm-unavailable-chip")].map((c) => c.textContent?.trim());
        expect(chips).toEqual(["Out of quota", "Signed out"]);
        expect(container.querySelector(".swarm-status-chip")).toBeNull();
    });
});

describe("AgentRow — the line says it too, and follows the clock", () => {
    afterEach(() => vi.useRealTimers());

    function lineOf(container: HTMLElement): string {
        return container.querySelector(".swarm-activity-summary")?.textContent?.trim() ?? "";
    }

    it("replaces the title with the same words as the chip", () => {
        seedBlockMeta({ "agent:last_failure": { code: "auth" } });
        const { container } = renderRow("idle");
        expect(lineOf(container)).toBe("Needs sign-in");
    });

    it("says out of quota until the reset, then goes back to the title when it passes", () => {
        vi.useFakeTimers();
        const resets = Date.now() + 90_000;
        seedBlockMeta({ "agent:last_failure": { code: "usage_limit", resetsAtMs: resets } });
        const { container } = renderRow("idle");
        expect(lineOf(container)).toMatch(/^Out of quota until /);
        vi.advanceTimersByTime(91_000);
        expect(lineOf(container)).toBe("No activity yet");
        expect(container.querySelector(".swarm-unavailable-chip")).toBeNull();
    });

    it("leaves the title alone for an agent that can work", () => {
        seedBlockMeta({});
        const { container } = renderRow("idle");
        expect(lineOf(container)).toBe("No activity yet");
    });
});
