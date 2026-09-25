// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Right-click "Copy ..." menus on the Swarm rows, end-to-end through `AgentRow`.
 *
 * Swarm rows are `user-select: none`, so the pane's generic Copy-on-selection
 * menu can never fire for them (docs/specs/REPORT_CONTEXT_MENU_GAP_AUDIT_
 * 2026_08_07.md section 4). The real ContextMenuModel + `showCopyContextMenu`
 * run here; only the native menu bridge (`getApi().showContextMenu`) and the
 * clipboard are mocked, so a click is exercised the way the host delivers it.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
    showContextMenu: vi.fn(),
    writeText: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/clipboard", () => ({ writeText: mocks.writeText, readText: () => Promise.resolve("") }));
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<Record<string, unknown>>()),
    getApi: () => ({ showContextMenu: mocks.showContextMenu }),
}));

import { __resetAllSlots, dispatch, registerPane } from "@/app/store/agent-document-store";
import { ContextMenuModel } from "@/app/store/contextmenu";
import type { DocumentNode } from "@/app/view/agent/types";
import type {
    ActiveCron,
    ActiveShell,
    ActiveSubagent,
    AgentTreeNode,
    SwarmViewModel,
    TodoItem,
    WorkflowDispatch,
} from "./swarm-model";
import { AgentRow } from "./swarm-view";

afterEach(() => {
    cleanup();
    __resetAllSlots();
    vi.clearAllMocks();
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

function modelStub(): SwarmViewModel {
    return {
        isAgentCollapsed: () => false,
        toggleAgentCollapsed: () => {},
        isSelected: () => false,
        toggleSelected: () => {},
        isExpanded: () => false,
        toggleDispatchExpanded: () => {},
        countdownStateAtom: () => new Map(),
        pauseCountdown: () => {},
        resumeCountdown: () => {},
        retireRow: () => {},
    } as unknown as SwarmViewModel;
}

/**
 * Renders the row inside a stand-in for the pane body's own menu handler.
 * `escaped.count` counts right-clicks that reached it, i.e. that would have
 * opened the generic Split/Replace/Magnify/Close menu. Solid delegates
 * `contextmenu` to the document and replays propagation itself, so a wrapper
 * Solid handler (not a raw DOM listener) is what observes stopPropagation.
 */
function renderRow(node: AgentTreeNode, opts: { seedPane?: boolean } = {}) {
    if (opts.seedPane !== false) registerPane(BLOCK);
    const escaped = { count: 0 };
    const result = render(() => (
        <div onContextMenu={() => escaped.count++}>
            <AgentRow node={node} focusedBlockId={() => null} model={modelStub()} />
        </div>
    ));
    return { ...result, escaped };
}

function rightClick(el: Element | null): MouseEvent {
    expect(el).not.toBeNull();
    const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 });
    el!.dispatchEvent(e);
    return e;
}

type NativeItem = { label: string; id: string };
function shownMenu(): NativeItem[] {
    expect(mocks.showContextMenu).toHaveBeenCalledOnce();
    return mocks.showContextMenu.mock.calls[0][1];
}
function labels(): string[] {
    return shownMenu().map((i) => i.label);
}
function clickItem(label: string) {
    const item = shownMenu().find((i) => i.label === label);
    expect(item, `menu item "${label}"`).toBeDefined();
    ContextMenuModel.handleContextMenuClick(item!.id);
}

const SHELL: ActiveShell = {
    shell_id: "sh_123",
    block_id: BLOCK,
    cmd: "npm run build -- --watch",
    title: "watcher",
    started_at: Date.now(),
    line_count: 0,
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
const SUB = {
    agent_id: "a1b2c3d4e5f6",
    slug: "Explore",
    display_name: null,
    dispatch_id: "solo:a1b2c3d4e5f6",
    status: "active",
    last_event_at: Date.now(),
} as unknown as ActiveSubagent;
const WORKFLOW: WorkflowDispatch = {
    kind: "workflowDispatch",
    dispatchId: "wf_77",
    name: "Audit sweep",
    memberCount: 3,
    membersDone: 1,
    status: "active",
    lastEventAt: Date.now(),
};
const TODOS: TodoItem[] = [{ text: "Wire the frontend", status: "pending" }];

describe("Swarm sub-row context menus", () => {
    it("shell row: copies the command, its distinct title, and the shell id; does not reach the pane menu", () => {
        const { container, escaped } = renderRow(treeNode({ shellRows: [SHELL] }));
        const e = rightClick(container.querySelector(".swarm-shell-row"));
        expect(e.defaultPrevented).toBe(true);
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy command", "Copy title", "Copy shell ID"]);
        clickItem("Copy command");
        expect(mocks.writeText).toHaveBeenLastCalledWith("npm run build -- --watch");
        clickItem("Copy title");
        expect(mocks.writeText).toHaveBeenLastCalledWith("watcher");
        clickItem("Copy shell ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith("sh_123");
    });

    it("shell row: omits the title entry when it merely defaults to the command", () => {
        const { container } = renderRow(treeNode({ shellRows: [{ ...SHELL, title: SHELL.cmd }] }));
        rightClick(container.querySelector(".swarm-shell-row"));
        expect(labels()).toEqual(["Copy command", "Copy shell ID"]);
    });

    it("cron row: copies name, schedule, target agent and job id", () => {
        const { container, escaped } = renderRow(treeNode({ cronRows: [CRON] }));
        rightClick(container.querySelector(".swarm-cron-row"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy name", "Copy schedule", "Copy target agent", "Copy cron ID"]);
        clickItem("Copy schedule");
        expect(mocks.writeText).toHaveBeenLastCalledWith("0 3 * * *");
        clickItem("Copy cron ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith("cron_9");
        clickItem("Copy target agent");
        expect(mocks.writeText).toHaveBeenLastCalledWith("agent2");
        clickItem("Copy name");
        expect(mocks.writeText).toHaveBeenLastCalledWith("nightly check");
    });

    it("subagent row: copies the shown name, the slug (when distinct) and the agent id", () => {
        const { container, escaped } = renderRow(treeNode({ agentToolRows: [SUB] }));
        rightClick(container.querySelector(".swarm-subagent-row"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy name", "Copy slug", "Copy agent ID"]);
        clickItem("Copy name");
        expect(mocks.writeText).toHaveBeenLastCalledWith("Explore · a1b2c3d");
        clickItem("Copy slug");
        expect(mocks.writeText).toHaveBeenLastCalledWith("Explore");
        clickItem("Copy agent ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith("a1b2c3d4e5f6");
    });

    it("subagent row: with a resolved display name, name and slug are separate copies", () => {
        const { container } = renderRow(treeNode({ agentToolRows: [{ ...SUB, display_name: "Map the repo" }] }));
        rightClick(container.querySelector(".swarm-subagent-row"));
        clickItem("Copy name");
        expect(mocks.writeText).toHaveBeenLastCalledWith("Map the repo");
        clickItem("Copy slug");
        expect(mocks.writeText).toHaveBeenLastCalledWith("Explore");
    });

    it("subagent row: no slug entry when the slug is empty", () => {
        const { container } = renderRow(treeNode({ agentToolRows: [{ ...SUB, slug: "" }] }));
        rightClick(container.querySelector(".swarm-subagent-row"));
        expect(labels()).toEqual(["Copy name", "Copy agent ID"]);
    });

    it("workflow row: copies the name and the workflow id", () => {
        const { container, escaped } = renderRow(treeNode({ workflowRows: [WORKFLOW] }));
        rightClick(container.querySelector(".swarm-workflow-header"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy name", "Copy workflow ID"]);
        clickItem("Copy name");
        expect(mocks.writeText).toHaveBeenLastCalledWith("Audit sweep");
        clickItem("Copy workflow ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith("wf_77");
    });

    it("todo row: copies the todo text", () => {
        const { container, escaped } = renderRow(treeNode({ todoRows: TODOS }));
        rightClick(container.querySelector(".swarm-todo-row"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy todo text"]);
        clickItem("Copy todo text");
        expect(mocks.writeText).toHaveBeenLastCalledWith("Wire the frontend");
    });

    it("long-running row: copies the command", () => {
        registerPane(BLOCK);
        const doc = {
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
        dispatch(BLOCK, { type: "StreamFlush", newNodes: [doc], updatedNodes: [] }, "system");
        const { container, escaped } = renderRow(treeNode(), { seedPane: false });
        rightClick(container.querySelector(".swarm-longrunning-row"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy command"]);
        clickItem("Copy command");
        expect(mocks.writeText).toHaveBeenLastCalledWith("sleep 300");
    });

    it("primary agent row keeps its menu (name + block id) after moving onto the shared helper", () => {
        const { container, escaped } = renderRow(treeNode());
        rightClick(container.querySelector(".swarm-agent-card"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy agent name", "Copy block ID"]);
        clickItem("Copy agent name");
        expect(mocks.writeText).toHaveBeenLastCalledWith("AgentX");
        clickItem("Copy block ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith(BLOCK);
    });
});
