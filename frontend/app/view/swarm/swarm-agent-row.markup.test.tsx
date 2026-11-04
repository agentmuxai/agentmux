// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent's row in the Swarm draws, pinned as markup: its card (checkbox, chevron, name, marks,
 * chip, line, current tool), the "no pane" mark, and the buckets under it. A refactor of the row must
 * leave the committed snapshots as they are; regenerate with `vitest -u` only for a change meant to
 * look different. The click, press and key behaviour is pinned in `swarm-agent-row.render.test.tsx`.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { DockNodeStatusCommand: () => Promise.resolve() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/agent-runtime-live", () => ({ openAgentInActiveTab: async () => undefined }));

import { __resetAllSlots, registerPane } from "@/app/store/agent-document-store";
import { MOS } from "@/app/store/global";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { ActiveCron, ActiveShell, ActiveSubagent, AgentTreeNode, SwarmViewModel, WorkflowDispatch } from "./swarm-model";
import { AgentRow } from "./swarm-view";

const NOW = 1_800_000_000_000;
const BLOCK = "mk1";

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
});
afterEach(() => {
    cleanup();
    __resetAllSlots();
    vi.useRealTimers();
});

let version = 0;
function seedBlockMeta(meta: Record<string, unknown>): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid: BLOCK,
        obj: { otype: "block", oid: BLOCK, version: ++version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

function treeNode(over: Partial<AgentTreeNode> = {}): AgentTreeNode {
    return {
        blockId: BLOCK,
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
        ...over,
    } as AgentTreeNode;
}

function modelOf(collapsed: boolean): SwarmViewModel {
    return {
        isAgentCollapsed: () => collapsed,
        toggleAgentCollapsed: () => {},
        isSelected: () => false,
        toggleSelected: () => {},
        countdownStateAtom: () => new Map(),
        isExpanded: () => false,
        getDispatchDetail: () => undefined,
        toggleDispatchExpanded: () => {},
        pauseCountdown: () => {},
        resumeCountdown: () => {},
        retireRow: () => {},
    } as unknown as SwarmViewModel;
}

const SUB = {
    agent_id: "a1b2c3d4e5f6",
    slug: "Explore",
    display_name: null,
    dispatch_id: "solo:a1b2c3d4e5f6",
    status: "active",
    last_event_at: NOW,
} as unknown as ActiveSubagent;
const WORKFLOW: WorkflowDispatch = {
    kind: "workflowDispatch",
    dispatchId: "wf_77",
    name: "Audit sweep",
    memberCount: 3,
    membersDone: 1,
    status: "active",
    lastEventAt: NOW,
};
const SHELL: ActiveShell = {
    shell_id: "sh_123",
    block_id: BLOCK,
    cmd: "npm run build -- --watch",
    title: "watcher",
    started_at: NOW - 90_000,
    line_count: 4,
};
const CRON: ActiveCron = {
    id: "cron_9",
    block_id: BLOCK,
    name: "nightly check",
    expression: "0 3 * * *",
    target: "agent2",
    created_by: "agent1",
    enabled: true,
    last_fired: null,
    fire_count: 0,
    max_fires: null,
    next_fire: null,
};
const BUSY = (): AgentTreeNode =>
    treeNode({
        activitySummary: "Reworking the parser",
        line: { text: "Reworking the parser", source: "declared", setAt: NOW - 600_000, ageMs: 600_000, stale: true },
        contextTokens: 123_456,
        currentTool: "Bash: npm test",
        todoRows: [
            { text: "Read the spec", status: "completed" },
            { text: "Write the parser", status: "in_progress" },
        ],
        agentToolRows: [SUB],
        workflowRows: [WORKFLOW],
        shellRows: [SHELL],
        cronRows: [CRON],
    });

/** One element per line, so a snapshot diff points at the element that changed. The last-active time
 *  of day depends on the machine's zone and is blanked. */
const markup = (el: Element | null): string =>
    (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<").replace(/Last active \d{1,2}:\d{2}:\d{2} (AM|PM)/g, "Last active <time>") + "\n";
const snap = (name: string) => `./__snapshots__/swarm-agent-row.${name}.html`;

function mount(node: AgentTreeNode, collapsed = false, focused: string | null = null) {
    registerPane(BLOCK);
    return render(() => (
        <div class="swarm-view" tabIndex={-1}>
            <AgentRow node={() => node} focusedBlockId={() => focused} model={modelOf(collapsed)} rowKey="mk-row" />
        </div>
    ));
}

describe("AgentRow markup", () => {
    it("a quiet agent: no chevron, a spacer, the status chip and the placeholder line", async () => {
        seedBlockMeta({});
        const v = mount(treeNode({ agentStatus: "idle" }));
        await expect(markup(v.container.querySelector(".swarm-agent-group"))).toMatchFileSnapshot(snap("quiet"));
    });

    it("a busy agent, expanded: every bucket under the card, and the current tool on it", async () => {
        seedBlockMeta({ "session:last_activity_ms": NOW - 7 * 60_000 });
        const v = mount(BUSY(), false, BLOCK);
        await expect(markup(v.container.querySelector(".swarm-agent-group"))).toMatchFileSnapshot(snap("busy-expanded"));
    });

    it("a busy agent, collapsed: the card with the count of what is hidden, and no buckets", async () => {
        seedBlockMeta({});
        const v = mount(BUSY(), true);
        await expect(markup(v.container.querySelector(".swarm-agent-group"))).toMatchFileSnapshot(snap("busy-collapsed"));
    });

    it("an agent with no pane: the mark, and the hover title that says a click opens it", async () => {
        seedBlockMeta({ "agent:parked": true, agentId: "uid-1" });
        const v = mount(treeNode({ agentStatus: "idle" }), true);
        await expect(markup(v.container.querySelector(".swarm-agent-group"))).toMatchFileSnapshot(snap("parked"));
    });

    it("an agent that can't start a turn: the chip and the line say so, muted", async () => {
        seedBlockMeta({ "agent:last_failure": { code: "auth" } });
        const v = mount(treeNode({ agentStatus: "idle" }), true);
        await expect(markup(v.container.querySelector(".swarm-agent-group"))).toMatchFileSnapshot(snap("unavailable"));
    });

    it("an agent with no block: a spacer for the checkbox, and no controls that need an id", async () => {
        const v = render(() => (
            <AgentRow node={() => treeNode({ blockId: null as unknown as string })} focusedBlockId={() => null} model={modelOf(false)} />
        ));
        await expect(markup(v.container.querySelector(".swarm-agent-group"))).toMatchFileSnapshot(snap("no-block"));
    });
});

describe("AgentRow summary flash", () => {
    beforeEach(() => {
        vi.useRealTimers();
        vi.useFakeTimers();
        vi.setSystemTime(NOW);
    });

    it("flashes when the summary changes, not for the one it mounts with, and stops after the flash", () => {
        seedBlockMeta({});
        registerPane(BLOCK);
        const [node, setNode] = createSignal(treeNode({ activitySummary: "First", agentStatus: "idle" }));
        const v = render(() => <AgentRow node={node} focusedBlockId={() => null} model={modelOf(true)} />);
        const summary = () => v.container.querySelector(".swarm-activity-summary")!;
        expect(summary().classList.contains("swarm-activity-summary--flash")).toBe(false);

        setNode(treeNode({ activitySummary: "Second", agentStatus: "idle" }));
        expect(summary().classList.contains("swarm-activity-summary--flash")).toBe(true);

        vi.advanceTimersByTime(5000);
        expect(summary().classList.contains("swarm-activity-summary--flash")).toBe(false);

        setNode(treeNode({ activitySummary: "Second", agentStatus: "running" }));
        expect(summary().classList.contains("swarm-activity-summary--flash")).toBe(false);
    });
});
